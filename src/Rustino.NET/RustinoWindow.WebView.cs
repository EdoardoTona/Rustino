using System.Runtime.InteropServices;
using System.Text.Json;

namespace Rustino.NET;

// Webview features for apps rather than browsers: native paths of dropped files, downloads,
// the browser's context menu and shortcuts, the document title, cookies.
public partial class RustinoWindow
{
    private bool _contextMenuEnabled = true;
    private bool _browserControlsEnabled = true;
    private ScrollBarStyle _scrollBarStyle;
    private bool _acceptFirstMouse;
    private bool _backForwardGestures;
    private bool _fileDropEnabled;

    private readonly EventObservable<FileDropEventArgs> _fileDropObs = new();
    private readonly EventObservable<string> _documentTitleChangedObs = new();
    private readonly EventObservable<DownloadStartingEventArgs> _downloadStartingObs = new();
    private readonly EventObservable<DownloadCompletedEventArgs> _downloadCompletedObs = new();

    private static readonly FileDropCallback FileDropCb = OnFileDropNative;
    private static readonly StringCallback DocumentTitleChangedCb = OnDocumentTitleChangedNative;
    private static readonly DownloadStartingCallback DownloadStartingCb = OnDownloadStartingNative;
    private static readonly DownloadCompletedCallback DownloadCompletedCb = OnDownloadCompletedNative;

    /// <summary>
    /// Files dragged from the desktop or a file manager move over the page, are dropped or leave it, with their full
    /// paths (HTML5 drag and drop only gives the page the file names). Needs <see cref="SetFileDropEnabled"/>.
    /// </summary>
    public event EventHandler<FileDropEventArgs>? FileDrop;

    /// <summary>
    /// The page's <c>&lt;title&gt;</c> changed. To show it in the title bar:
    /// <c>window.DocumentTitleChanged += (_, title) =&gt; window.SetTitle(title);</c>
    /// </summary>
    public event EventHandler<string>? DocumentTitleChanged;

    /// <summary>
    /// A download starts. Set <c>Cancel</c> to refuse it, or <c>DestinationPath</c> to save it without asking;
    /// otherwise the user chooses where in the native save dialog. Don't show dialogs from this handler.
    /// </summary>
    public event EventHandler<DownloadStartingEventArgs>? DownloadStarting;

    /// <summary>A download started with <see cref="DownloadStarting"/> ended: saved, failed or canceled by the user.</summary>
    public event EventHandler<DownloadCompletedEventArgs>? DownloadCompleted;

    public IObservable<FileDropEventArgs> WhenFileDrop => _fileDropObs;
    public IObservable<string> WhenDocumentTitleChanged => _documentTitleChangedObs;
    public IObservable<DownloadStartingEventArgs> WhenDownloadStarting => _downloadStartingObs;
    public IObservable<DownloadCompletedEventArgs> WhenDownloadCompleted => _downloadCompletedObs;

    /// <summary>
    /// The browser's context menu on right click (on by default). The page still gets the <c>contextmenu</c> event,
    /// e.g. to call <see cref="ShowContextMenu"/>. Must be called before <see cref="WaitForClose"/>.
    /// </summary>
    public RustinoWindow SetContextMenuEnabled(bool enabled)
    {
        _contextMenuEnabled = enabled;
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_set_context_menu_enabled(_nativeHandle, enabled ? 1 : 0);
        return this;
    }

    /// <summary>
    /// Windows: the browser's shortcuts, such as F5 and Ctrl+R (reload), Ctrl+F (find), Ctrl+P (print) and F12
    /// (on by default). Menu accelerators keep working. WKWebView and WebKitGTK have no such shortcuts. Must be
    /// called before <see cref="WaitForClose"/>.
    /// </summary>
    public RustinoWindow SetBrowserControlsEnabled(bool enabled)
    {
        _browserControlsEnabled = enabled;
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_set_browser_accelerator_keys_enabled(_nativeHandle, enabled ? 1 : 0);
        return this;
    }

    /// <summary>
    /// Windows: the style of the page's scroll bars. Windows that share a user data folder need the same style.
    /// Must be called before <see cref="WaitForClose"/>.
    /// </summary>
    public RustinoWindow SetScrollBarStyle(ScrollBarStyle style)
    {
        _scrollBarStyle = style;
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_set_scroll_bar_style(_nativeHandle, (int)style);
        return this;
    }

    /// <summary>
    /// macOS: the click that activates the window also reaches the page, instead of only activating the window.
    /// Must be called before <see cref="WaitForClose"/>.
    /// </summary>
    public RustinoWindow SetAcceptFirstMouse(bool accept)
    {
        _acceptFirstMouse = accept;
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_set_accept_first_mouse(_nativeHandle, accept ? 1 : 0);
        return this;
    }

    /// <summary>
    /// Horizontal swipes on the trackpad or the touch screen go back and forward in the history (off by default).
    /// Must be called before <see cref="WaitForClose"/>.
    /// </summary>
    public RustinoWindow SetBackForwardGesturesEnabled(bool enabled)
    {
        _backForwardGestures = enabled;
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_set_back_forward_gestures_enabled(_nativeHandle, enabled ? 1 : 0);
        return this;
    }

    /// <summary>
    /// Files dropped on the window raise <see cref="FileDrop"/> instead of reaching the page (off by default): the
    /// page doesn't get them, and the webview doesn't open them. On Windows the page's HTML5 drag and drop stops
    /// working altogether, even within the page. Must be called before <see cref="WaitForClose"/>.
    /// </summary>
    public RustinoWindow SetFileDropEnabled(bool enabled)
    {
        _fileDropEnabled = enabled;
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_set_file_drop_enabled(_nativeHandle, enabled ? 1 : 0);
        return this;
    }

    /// <summary>
    /// Opens the system print dialog for the page, like the page's <c>window.print()</c> (which Rustino sends to the
    /// system dialog on Windows, instead of WebView2's preview inside the window, and on macOS, where WKWebView ignores it).
    /// </summary>
    public RustinoWindow Print()
    {
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_print(_nativeHandle);
        return this;
    }

    /// <summary>Reloads the page. A page loaded from an HTML string is loaded again from the string.</summary>
    public RustinoWindow Reload()
    {
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_reload(_nativeHandle);
        return this;
    }

    /// <summary>
    /// Opens the web inspector; needs <see cref="SetDevToolsEnabled"/>. On macOS it also needs the native library
    /// built with the <c>devtools</c> feature, which uses a private WebKit API: without it the page can be inspected
    /// from Safari's Develop menu.
    /// </summary>
    public RustinoWindow OpenDevTools()
    {
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_open_devtools(_nativeHandle);
        return this;
    }

    /// <summary>Closes the web inspector (not supported on Windows; on macOS it needs the <c>devtools</c> feature).</summary>
    public RustinoWindow CloseDevTools()
    {
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_close_devtools(_nativeHandle);
        return this;
    }

    /// <summary>Deletes the cookies, the cache and the storage of every site.</summary>
    public RustinoWindow ClearBrowsingData()
    {
        if (_nativeHandle != IntPtr.Zero)
            RustinoWebViewDllImports.rustino_clear_browsing_data(_nativeHandle);
        return this;
    }

    /// <summary>
    /// The webview's cookies: all of them, or those it sends to <paramref name="url"/>. Empty before the window runs.
    /// </summary>
    /// <exception cref="InvalidOperationException">
    /// On Windows, called within a webview event (<see cref="WebMessageReceived"/>, <see cref="Navigating"/>,
    /// <see cref="PageLoaded"/>, ...): WebView2 answers only after the event. Call it once the handler returned,
    /// e.g. <c>await Task.Run(() =&gt; window.GetCookies())</c> in an async handler.
    /// </exception>
    public RustinoCookie[] GetCookies(string? url = null)
    {
        if (_nativeHandle == IntPtr.Zero) return [];
        var json = ConsumeStringResult(RustinoWebViewDllImports.rustino_get_cookies(_nativeHandle, url, out var status));
        if (status == 2)
            throw new InvalidOperationException(
                "On Windows, GetCookies can't run within a webview event: WebView2 answers only after the event returns.");
        if (json == null) return [];
        return JsonSerializer.Deserialize(json, CookieJsonContext.Default.RustinoCookieArray) ?? [];
    }

    /// <summary>
    /// Adds a cookie, or replaces the one with the same name, domain and path. Returns false if the window doesn't run.
    /// On Windows WebView2 applies it a moment later: <see cref="GetCookies"/> right after may not return it yet.
    /// </summary>
    public bool SetCookie(RustinoCookie cookie)
    {
        ArgumentNullException.ThrowIfNull(cookie);
        if (_nativeHandle == IntPtr.Zero) return false;
        var json = JsonSerializer.Serialize(cookie, CookieJsonContext.Default.RustinoCookie);
        return RustinoWebViewDllImports.rustino_set_cookie(_nativeHandle, json) != 0;
    }

    /// <summary>
    /// Deletes the cookie with the same name, domain and path. Returns false if the window doesn't run.
    /// On Windows WebView2 applies it a moment later: <see cref="GetCookies"/> right after may still return it.
    /// </summary>
    public bool DeleteCookie(RustinoCookie cookie)
    {
        ArgumentNullException.ThrowIfNull(cookie);
        if (_nativeHandle == IntPtr.Zero) return false;
        var json = JsonSerializer.Serialize(cookie, CookieJsonContext.Default.RustinoCookie);
        return RustinoWebViewDllImports.rustino_delete_cookie(_nativeHandle, json) != 0;
    }

    // Called by EnsureNative: settings made before the native window existed
    private void ApplyWebViewConfiguration()
    {
        if (!_contextMenuEnabled)
            RustinoWebViewDllImports.rustino_set_context_menu_enabled(_nativeHandle, 0);
        if (!_browserControlsEnabled)
            RustinoWebViewDllImports.rustino_set_browser_accelerator_keys_enabled(_nativeHandle, 0);
        if (_scrollBarStyle != ScrollBarStyle.Default)
            RustinoWebViewDllImports.rustino_set_scroll_bar_style(_nativeHandle, (int)_scrollBarStyle);
        if (_acceptFirstMouse)
            RustinoWebViewDllImports.rustino_set_accept_first_mouse(_nativeHandle, 1);
        if (_backForwardGestures)
            RustinoWebViewDllImports.rustino_set_back_forward_gestures_enabled(_nativeHandle, 1);
        if (_fileDropEnabled)
            RustinoWebViewDllImports.rustino_set_file_drop_enabled(_nativeHandle, 1);
    }

    // Called by RegisterCallbacks
    private void RegisterWebViewCallbacks()
    {
        RustinoWebViewDllImports.rustino_set_file_drop_handler(_nativeHandle, FileDropCb);
        RustinoWebViewDllImports.rustino_set_document_title_changed_handler(_nativeHandle, DocumentTitleChangedCb);
        RustinoWebViewDllImports.rustino_set_download_starting_handler(_nativeHandle, DownloadStartingCb);
        RustinoWebViewDllImports.rustino_set_download_completed_handler(_nativeHandle, DownloadCompletedCb);
    }

    // Called by CompleteAllObservables
    private static void CompleteWebViewObservables(RustinoWindow w)
    {
        w._fileDropObs.Complete();
        w._documentTitleChangedObs.Complete();
        w._downloadStartingObs.Complete();
        w._downloadCompletedObs.Complete();
    }

    private static void OnFileDropNative(IntPtr ctx, int kind, IntPtr pathsPtr, int x, int y)
    {
        if (!Instances.TryGetValue(ctx, out var w)) return;
        var paths = Marshal.PtrToStringUTF8(pathsPtr)?.Split('\n', StringSplitOptions.RemoveEmptyEntries) ?? [];
        var args = new FileDropEventArgs((FileDropEventType)kind, paths, x, y);
        w.FileDrop?.Invoke(w, args);
        w._fileDropObs.Emit(args);
    }

    private static void OnDocumentTitleChangedNative(IntPtr ctx, IntPtr titlePtr)
    {
        if (!Instances.TryGetValue(ctx, out var w)) return;
        var title = Marshal.PtrToStringUTF8(titlePtr) ?? "";
        w.DocumentTitleChanged?.Invoke(w, title);
        w._documentTitleChangedObs.Emit(title);
    }

    private static int OnDownloadStartingNative(IntPtr ctx, IntPtr urlPtr, IntPtr suggestedPathPtr, IntPtr response)
    {
        if (!Instances.TryGetValue(ctx, out var w)) return 1;
        var args = new DownloadStartingEventArgs(
            Marshal.PtrToStringUTF8(urlPtr) ?? "", Marshal.PtrToStringUTF8(suggestedPathPtr) ?? "");
        w.DownloadStarting?.Invoke(w, args);
        w._downloadStartingObs.Emit(args);
        if (args.Cancel) return 0;
        if (!string.IsNullOrEmpty(args.DestinationPath))
        {
            string destination;
            try { destination = Path.GetFullPath(args.DestinationPath); }
            catch (Exception) { return 0; } // not a path: nowhere to save the file
            RustinoWebViewDllImports.rustino_set_download_destination(response, destination);
        }
        return 1;
    }

    private static void OnDownloadCompletedNative(IntPtr ctx, IntPtr urlPtr, IntPtr pathPtr, int success)
    {
        if (!Instances.TryGetValue(ctx, out var w)) return;
        var args = new DownloadCompletedEventArgs(
            Marshal.PtrToStringUTF8(urlPtr) ?? "", success != 0 ? Marshal.PtrToStringUTF8(pathPtr) : null);
        w.DownloadCompleted?.Invoke(w, args);
        w._downloadCompletedObs.Emit(args);
    }
}
