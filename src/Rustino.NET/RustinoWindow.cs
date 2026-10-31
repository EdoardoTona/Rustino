using System.Collections.Concurrent;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text.Json;
using Microsoft.Extensions.Logging;

namespace Rustino.NET;

public partial class RustinoWindow : IDisposable
{
    // Set once, under _lifecycleLock, when the native window is created; Dispose disposes it but
    // never resets it
    private RustinoWindowHandle? _nativeHandle;
    private int _disposed;
    private bool _runStarted;
    private readonly object _lifecycleLock = new();
    private ILogger? _logger;
    private int _logVerbosity;

    // Original configuration fields
    private string _title = "Rustino Window";
    private int _width = 800;
    private int _height = 600;
    private bool _resizable = true;
    private bool _topmost;
    private bool _useOsDefaultSize = true;
    private bool _devToolsEnabled;
    private bool _clipboardEnabled;
    private bool _ignoreCertErrors;
    private bool _webSecurityEnabled = true;
    private string? _iconFile;
    // Icons that SetIcon and SetTrayIcon wrote from a stream
    private string? _iconTempFile;
    private string? _trayIconTempFile;
    private bool _center;

    // New configuration fields
    private bool _transparent;
    private bool _decorations = true;
    private bool _visible = true;
    private bool _maximized;
    private bool _fullscreen;
    private (int X, int Y)? _position;
    private (int Width, int Height)? _minSize;
    private (int Width, int Height)? _maxSize;
    private (byte R, byte G, byte B, byte A)? _backgroundColor;
    private string? _userAgent;
    private string? _userDataFolder;
    private bool _mediaAutoplay = true;
    private bool _zoomHotkeys;
    private readonly List<string> _initScripts = new();
    private readonly Dictionary<string, NetCustomSchemeDelegate> _customSchemes = new(StringComparer.OrdinalIgnoreCase);

    // About metadata fields
    private string? _aboutName;
    private string? _aboutVersion;
    private string? _aboutCopyright;
    private string? _aboutWebsite;
    private string? _aboutLicense;
    private readonly List<string> _aboutAuthors = new();
    private string? _aboutComments;

    // Observable streams for reactive consumers
    private readonly EventObservable<(int Width, int Height)> _sizeChangedObs = new();
    private readonly EventObservable<(int X, int Y)> _locationChangedObs = new();
    private readonly EventObservable<bool> _focusChangedObs = new();
    private readonly EventObservable<string> _webMessageObs = new();
    private readonly EventObservable<WebMessageEventArgs> _webMessageWithSourceObs = new();
    private readonly EventObservable<PageLoadEventArgs> _pageLoadedObs = new();
    private readonly EventObservable<NavigationEventArgs> _navigatingObs = new();
    private readonly EventObservable<EventArgs> _windowClosedObs = new();
    private readonly EventObservable<string> _menuItemClickedObs = new();
    private readonly EventObservable<MenuItemCheckedEventArgs> _menuItemCheckedChangedObs = new();
    private readonly EventObservable<TrayIconClickedEventArgs> _trayIconClickedObs = new();

    /// <summary>Native log level. Set this before <see cref="Load(string)"/>; no native setter exists.</summary>
    public int LogVerbosity
    {
        get => _logVerbosity;
        set
        {
            lock (_lifecycleLock)
            {
                ThrowIfDisposed();
                if (_nativeHandle != null)
                    throw CreationSettingStarted(nameof(LogVerbosity));
                _logVerbosity = value;
            }
        }
    }

    // --- Instance routing for callbacks ---
    private static readonly ConcurrentDictionary<IntPtr, RustinoWindow> Instances = new();

    // Static delegate instances pinned by static fields (prevents GC)
    private static readonly ClosingCallback ClosingCb = OnClosingNative;
    private static readonly VoidContextCallback ClosedCb = OnClosedNative;
    private static readonly SizeCallback ResizedCb = OnResizedNative;
    private static readonly PointCallback MovedCb = OnMovedNative;
    private static readonly IntCallback FocusCb = OnFocusChangedNative;
    private static readonly WebMessageCallback WebMsgCb = OnWebMessageNative;
    private static readonly PageLoadCallback PageLoadCb = OnPageLoadNative;
    private static readonly NavigationCallback NavCb = OnNavigationNative;
    private static readonly MenuItemCallback MenuItemCb = OnMenuItemClickedNative;
    private static readonly TrayIconCallback TrayCb = OnTrayIconClickedNative;
    private static readonly CustomSchemeCallback CustomSchemeCb = OnCustomSchemeNative;
    private static readonly LogCallback LogCb = OnLogMessageNative;

    // --- Logging delegate ---
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void LogCallback(IntPtr context, int level, IntPtr message);

    // --- Custom scheme delegate (same signature as Photino) ---
    public delegate Stream? NetCustomSchemeDelegate(object sender, string scheme, string url, out string? contentType);

    // --- Events ---

    public event EventHandler<CancelEventArgs>? WindowClosing;
    public event EventHandler? WindowClosed;
    /// <summary>A managed exception was caught in a callback invoked by the native window.</summary>
    public event EventHandler<NativeCallbackExceptionEventArgs>? UnhandledCallbackException;
    public event EventHandler<SizeEventArgs>? SizeChanged;
    public event EventHandler<PointEventArgs>? LocationChanged;
    public event EventHandler<bool>? FocusChanged;
    public event EventHandler<string>? WebMessageReceived;
    // Same messages with the URL of the sending page: check it before trusting a message
    // if the webview can navigate to other sites
    public event EventHandler<WebMessageEventArgs>? WebMessageReceivedWithSource;
    public event EventHandler<PageLoadEventArgs>? PageLoaded;
    public event EventHandler<NavigationEventArgs>? Navigating;
    public event EventHandler<string>? MenuItemClicked;
    // Raised after MenuItemClicked when the user toggles a check item
    public event EventHandler<MenuItemCheckedEventArgs>? MenuItemCheckedChanged;
    // Raised once per click on the tray icon: on release, or on press when the click opens the
    // tray menu. Never raised on Linux.
    public event EventHandler<TrayIconClickedEventArgs>? TrayIconClicked;

    // --- Observable streams ---

    public IObservable<(int Width, int Height)> WhenSizeChanged => _sizeChangedObs;
    public IObservable<(int X, int Y)> WhenLocationChanged => _locationChangedObs;
    public IObservable<bool> WhenFocusChanged => _focusChangedObs;
    public IObservable<string> WhenWebMessageReceived => _webMessageObs;
    public IObservable<WebMessageEventArgs> WhenWebMessageReceivedWithSource => _webMessageWithSourceObs;
    public IObservable<PageLoadEventArgs> WhenPageLoaded => _pageLoadedObs;
    public IObservable<NavigationEventArgs> WhenNavigating => _navigatingObs;
    public IObservable<EventArgs> WhenWindowClosed => _windowClosedObs;
    public IObservable<string> WhenMenuItemClicked => _menuItemClickedObs;
    public IObservable<MenuItemCheckedEventArgs> WhenMenuItemCheckedChanged => _menuItemCheckedChangedObs;
    public IObservable<TrayIconClickedEventArgs> WhenTrayIconClicked => _trayIconClickedObs;

    // --- Notifications (static — no window required) ---

    public static bool ShowNotification(string title, string body, string? iconPath = null, string? appId = null)
    {
        NativeLibraryResolver.EnsureRegistered();
        return RustinoDllImports.rustino_show_notification(title, body, iconPath, appId) != 0;
    }

    // Windows drops toasts sent with an appId it cannot resolve, and unpackaged apps
    // (plain .exe, dotnet tools) are not registered anywhere: call this once at startup
    // before passing that appId to ShowNotification. appId must be a valid AppUserModelID
    // (e.g. "Company.Product": at most 128 characters, no spaces or backslashes), otherwise
    // this returns false on every platform. The icon must be an image file (.png/.ico)
    // that stays at iconPath; omitting it removes a previously registered icon.
    // Nothing is written on macOS and Linux.
    public static bool RegisterNotificationAppId(string appId, string displayName, string? iconPath = null)
    {
        NativeLibraryResolver.EnsureRegistered();
        return RustinoDllImports.rustino_register_notification_app_id(appId, displayName, iconPath) != 0;
    }

    public static bool ShowNotification(string title, string body, Stream icon, string? appId = null)
    {
        var tempPath = Path.Combine(Path.GetTempPath(), $"rustino_notify_{Guid.NewGuid():N}.png");
        try
        {
            using (var fs = File.Create(tempPath))
                icon.CopyTo(fs);
            return ShowNotification(title, body, tempPath, appId);
        }
        finally
        {
            try { File.Delete(tempPath); } catch { }
        }
    }

    // --- Constructor ---

    public RustinoWindow()
    {
        NativeLibraryResolver.EnsureRegistered();
        _customSchemes[LocalFileScheme] = ServeLocalFile;
    }

    // --- Logger configuration ---

    public RustinoWindow SetLogger(ILogger logger)
    {
        ThrowIfDisposed();
        _logger = logger;
        return this;
    }

    // --- Original builder methods ---

    public RustinoWindow SetUseOsDefaultSize(bool useDefault)
    {
        SetCreationOnly(ref _useOsDefaultSize, useDefault, nameof(SetUseOsDefaultSize),
            static (instance, value) => RustinoDllImports.rustino_set_use_os_default_size(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetSize(int width, int height)
    {
        ThrowIfDisposed();
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(width);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(height);
        _width = width;
        _height = height;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_size(handle, width, height);
        return this;
    }

    public RustinoWindow SetTitle(string title)
    {
        ThrowIfDisposed();
        _title = title;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_title(handle, title);
        return this;
    }

    public RustinoWindow SetResizable(bool resizable)
    {
        ThrowIfDisposed();
        _resizable = resizable;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_resizable(handle, resizable ? 1 : 0);
        return this;
    }

    public RustinoWindow SetTopMost(bool topMost)
    {
        ThrowIfDisposed();
        _topmost = topMost;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_topmost(handle, topMost ? 1 : 0);
        return this;
    }

    public RustinoWindow SetJavascriptClipboardAccessEnabled(bool enabled)
    {
        SetCreationOnly(ref _clipboardEnabled, enabled, nameof(SetJavascriptClipboardAccessEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_clipboard_enabled(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetDevToolsEnabled(bool enabled)
    {
        SetCreationOnly(ref _devToolsEnabled, enabled, nameof(SetDevToolsEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_devtools_enabled(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetIgnoreCertificateErrorsEnabled(bool enabled)
    {
        SetCreationOnly(ref _ignoreCertErrors, enabled, nameof(SetIgnoreCertificateErrorsEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_ignore_cert_errors(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetWebSecurityEnabled(bool enabled)
    {
        SetCreationOnly(ref _webSecurityEnabled, enabled, nameof(SetWebSecurityEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_web_security_enabled(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetIconFile(string path) => SetIconFile(path, tempFile: false);

    // Written to a temporary file, deleted when the icon is replaced and by Dispose
    public RustinoWindow SetIcon(Stream icon)
    {
        ThrowIfDisposed();
        return SetIconFile(WriteTempFile(icon, "rustino_icon"), tempFile: true);
    }

    private RustinoWindow SetIconFile(string path, bool tempFile)
    {
        try
        {
            ThrowIfDisposed();
            _iconFile = path;
            if (Native is { } handle)
            {
                RustinoDllImports.rustino_set_icon_file(handle, path);
                if (OperatingSystem.IsMacOS())
                {
                    SetMacDockIcon(path);
                }
            }
        }
        catch
        {
            if (tempFile) DeleteFile(path);
            throw;
        }
        // The macOS Dock icon is loaded again from _iconFile at every page load
        DeleteFile(Interlocked.Exchange(ref _iconTempFile, tempFile ? path : null));
        return this;
    }

    public RustinoWindow Center()
    {
        ThrowIfDisposed();
        _center = true;
        if (Native is { } handle)
            RustinoDllImports.rustino_center(handle);
        return this;
    }

    /// <summary>
    /// Navigates to <paramref name="uri"/>. <c>file:</c> URIs and relative URIs are local files, as with
    /// <see cref="Load(string)"/>.
    /// </summary>
    public RustinoWindow Load(Uri uri)
    {
        ArgumentNullException.ThrowIfNull(uri);
        ThrowIfDisposed();
        var url = !uri.IsAbsoluteUri ? LocalFileUrl(ResolveFile(uri.OriginalString))
            : uri.IsFile && !uri.IsUnc ? LocalFileUrl(Path.GetFullPath(uri.LocalPath), uri.Query + uri.Fragment)
            : uri.AbsoluteUri;
        RustinoDllImports.rustino_navigate_to_url(EnsureNative(), url);
        return this;
    }

    /// <summary>
    /// Navigates to an absolute URL (<c>https:</c>, a custom scheme, ...), to the HTML of a
    /// <c>data:text/html,</c> URL, or to a local file (a path or a <c>file:</c> URL). A relative path is
    /// resolved against the current directory, or against the app's directory when the file isn't in the
    /// current directory. Local files are served from <c>rustino-file://localhost/&lt;full path&gt;</c>, with
    /// the other files of their folder and its subfolders: <c>file:</c> pages can't send messages to the host.
    /// </summary>
    public RustinoWindow Load(string pathOrUrl)
    {
        ArgumentException.ThrowIfNullOrEmpty(pathOrUrl);
        ThrowIfDisposed();
        if (pathOrUrl.StartsWith("data:text/html,", StringComparison.OrdinalIgnoreCase))
        {
            var html = Uri.UnescapeDataString(pathOrUrl["data:text/html,".Length..]);
            RustinoDllImports.rustino_navigate_to_string(EnsureNative(), html);
        }
        else
        {
            RustinoDllImports.rustino_navigate_to_url(EnsureNative(), ToUrl(pathOrUrl));
        }
        return this;
    }

    // Absolute URLs stay as they are; paths and file: URLs are local files. Absolute paths
    // (C:\app\index.html, /app/index.html) parse as file URIs too.
    internal string ToUrl(string pathOrUrl)
    {
        if (!Uri.TryCreate(pathOrUrl, UriKind.Absolute, out var uri) || (uri.IsFile && Path.IsPathRooted(pathOrUrl)))
            return LocalFileUrl(ResolveFile(pathOrUrl));
        return uri.IsFile && !uri.IsUnc ? LocalFileUrl(Path.GetFullPath(uri.LocalPath), uri.Query + uri.Fragment) : pathOrUrl;
    }

    private static string ResolveFile(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (!File.Exists(fullPath) && !Path.IsPathRooted(path))
        {
            var appPath = Path.GetFullPath(path, AppContext.BaseDirectory);
            if (File.Exists(appPath))
                fullPath = appPath;
        }
        return fullPath;
    }

    // --- New builder methods (Phase 3-5) ---

    public RustinoWindow SetTransparent(bool transparent)
    {
        SetCreationOnly(ref _transparent, transparent, nameof(SetTransparent),
            static (instance, value) => RustinoDllImports.rustino_set_transparent(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetChromeless(bool chromeless)
    {
        ThrowIfDisposed();
        _decorations = !chromeless;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_decorations(handle, _decorations ? 1 : 0);
        return this;
    }

    public RustinoWindow SetPosition(int x, int y)
    {
        ThrowIfDisposed();
        _position = (x, y);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_position(handle, x, y);
        return this;
    }

    public RustinoWindow SetMinSize(int width, int height)
    {
        ThrowIfDisposed();
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(width);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(height);
        _minSize = (width, height);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_min_size(handle, width, height);
        return this;
    }

    public RustinoWindow SetMaxSize(int width, int height)
    {
        ThrowIfDisposed();
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(width);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(height);
        _maxSize = (width, height);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_max_size(handle, width, height);
        return this;
    }

    public RustinoWindow SetBackgroundColor(byte r, byte g, byte b, byte a = 255)
    {
        ThrowIfDisposed();
        _backgroundColor = (r, g, b, a);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_background_color(handle, r, g, b, a);
        return this;
    }

    public RustinoWindow SetUserAgent(string userAgent)
    {
        SetCreationOnly(ref _userAgent, userAgent, nameof(SetUserAgent), RustinoDllImports.rustino_set_user_agent);
        return this;
    }

    public RustinoWindow SetUserDataFolder(string path)
    {
        SetCreationOnly(ref _userDataFolder, path, nameof(SetUserDataFolder), RustinoDllImports.rustino_set_user_data_folder);
        return this;
    }

    public RustinoWindow SetMediaAutoplayEnabled(bool enabled)
    {
        SetCreationOnly(ref _mediaAutoplay, enabled, nameof(SetMediaAutoplayEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_media_autoplay(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow SetZoomHotkeysEnabled(bool enabled)
    {
        SetCreationOnly(ref _zoomHotkeys, enabled, nameof(SetZoomHotkeysEnabled),
            static (instance, value) => RustinoDllImports.rustino_set_zoom_hotkeys(instance, value ? 1 : 0));
        return this;
    }

    public RustinoWindow AddInitScript(string script)
    {
        lock (_lifecycleLock)
        {
            ThrowIfDisposed();
            if (_runStarted)
                throw CreationSettingStarted(nameof(AddInitScript));
            if (Native is { } handle && RustinoDllImports.rustino_add_init_script(handle, script) == 0)
                throw CreationSettingStarted(nameof(AddInitScript));
            _initScripts.Add(script);
        }
        return this;
    }

    // Serves requests for `scheme://...` URLs from .NET. Must be registered before WaitForClose().
    // Returning null from the handler produces a 404 response.
    public RustinoWindow RegisterCustomSchemeHandler(string scheme, NetCustomSchemeDelegate handler)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(scheme);
        ArgumentNullException.ThrowIfNull(handler);
        scheme = scheme.ToLowerInvariant();
        if (scheme == LocalFileScheme)
            throw new ArgumentException($"The {LocalFileScheme} scheme serves the local files passed to Load.", nameof(scheme));
        lock (_lifecycleLock)
        {
            ThrowIfDisposed();
            if (_runStarted)
                throw CreationSettingStarted(nameof(RegisterCustomSchemeHandler));
            if (Native is { } handle
                && !_customSchemes.ContainsKey(scheme)
                && RustinoDllImports.rustino_add_custom_scheme(handle, scheme) == 0)
                throw CreationSettingStarted(nameof(RegisterCustomSchemeHandler));
            _customSchemes[scheme] = handler;
        }
        return this;
    }

    public RustinoWindow SetAboutName(string name)
    {
        ThrowIfDisposed();
        _aboutName = name;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_name(handle, name);
        return this;
    }

    public RustinoWindow SetAboutVersion(string version)
    {
        ThrowIfDisposed();
        _aboutVersion = version;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_version(handle, version);
        return this;
    }

    public RustinoWindow SetAboutCopyright(string copyright)
    {
        ThrowIfDisposed();
        _aboutCopyright = copyright;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_copyright(handle, copyright);
        return this;
    }

    public RustinoWindow SetAboutWebsite(string website)
    {
        ThrowIfDisposed();
        _aboutWebsite = website;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_website(handle, website);
        return this;
    }

    public RustinoWindow SetAboutLicense(string license)
    {
        ThrowIfDisposed();
        _aboutLicense = license;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_license(handle, license);
        return this;
    }

    public RustinoWindow AddAboutAuthor(string author)
    {
        ThrowIfDisposed();
        _aboutAuthors.Add(author);
        if (Native is { } handle)
        {
            var authorsStr = string.Join("\n", _aboutAuthors);
            RustinoDllImports.rustino_set_about_authors(handle, authorsStr);
        }
        return this;
    }

    public RustinoWindow SetAboutComments(string comments)
    {
        ThrowIfDisposed();
        _aboutComments = comments;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_about_comments(handle, comments);
        return this;
    }

    // --- Window state (post-run operations) ---

    public RustinoWindow SetMaximized(bool maximized)
    {
        ThrowIfDisposed();
        _maximized = maximized;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_maximized(handle, maximized ? 1 : 0);
        return this;
    }

    public RustinoWindow Minimize()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_minimized(handle, 1);
        return this;
    }

    public RustinoWindow Maximize()
    {
        ThrowIfDisposed();
        _maximized = true;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_maximized(handle, 1);
        return this;
    }

    public RustinoWindow Restore()
    {
        ThrowIfDisposed();
        _maximized = false;
        if (Native is { } handle)
        {
            RustinoDllImports.rustino_set_minimized(handle, 0);
            RustinoDllImports.rustino_set_maximized(handle, 0);
        }
        return this;
    }

    public RustinoWindow SetFullscreen(bool fullscreen)
    {
        ThrowIfDisposed();
        _fullscreen = fullscreen;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_fullscreen(handle, fullscreen ? 1 : 0);
        return this;
    }

    public RustinoWindow SetVisible(bool visible)
    {
        ThrowIfDisposed();
        _visible = visible;
        if (Native is { } handle)
            RustinoDllImports.rustino_set_visible(handle, visible ? 1 : 0);
        return this;
    }

    public RustinoWindow Focus()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_focus(handle);
        return this;
    }

    public bool IsMinimized =>
        Native is { } handle && RustinoDllImports.rustino_is_minimized(handle) != 0;

    public bool IsMaximized =>
        Native is { } handle && RustinoDllImports.rustino_is_maximized(handle) != 0;

    public bool IsFullscreen =>
        Native is { } handle && RustinoDllImports.rustino_is_fullscreen(handle) != 0;

    public (int X, int Y) GetPosition()
    {
        if (Native is not { } handle) return (0, 0);
        RustinoDllImports.rustino_get_position(handle, out var x, out var y);
        return (x, y);
    }

    public (int Width, int Height) GetSize()
    {
        if (Native is not { } handle) return (_width, _height);
        RustinoDllImports.rustino_get_size(handle, out var w, out var h);
        return (w, h);
    }

    // --- WebView operations (post-run) ---

    public RustinoWindow ExecuteScript(string script)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_evaluate_script(handle, script);
        return this;
    }

    public RustinoWindow SendWebMessage(string message)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_send_web_message(handle, message);
        return this;
    }

    public RustinoWindow SetZoom(double factor)
    {
        ThrowIfDisposed();
        if (factor <= 0 || !double.IsFinite(factor))
            throw new ArgumentOutOfRangeException(nameof(factor), "Zoom factor must be a positive finite number.");
        if (Native is { } handle)
            RustinoDllImports.rustino_set_zoom(handle, factor);
        return this;
    }

    // --- Monitors (post-run) ---

    public MonitorInfo[] GetMonitors()
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return [];
        var ptr = RustinoDllImports.rustino_get_monitors(handle);
        var json = ConsumeStringResult(ptr);
        if (json == null) return [];
        return JsonSerializer.Deserialize(json, MonitorJsonContext.Default.MonitorInfoArray) ?? [];
    }

    public MonitorInfo? GetCurrentMonitor()
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        var ptr = RustinoDllImports.rustino_get_current_monitor(handle);
        var json = ConsumeStringResult(ptr);
        if (json == null) return null;
        return JsonSerializer.Deserialize(json, MonitorJsonContext.Default.MonitorInfo);
    }

    // --- Menus (post-run) ---

    public RustinoWindow SetMenu(RustinoMenu menu)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu(handle, menu.ToJson());
        return this;
    }

    public RustinoWindow RemoveMenu()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_remove_menu(handle);
        return this;
    }

    public RustinoWindow ShowContextMenu(RustinoMenu menu, double? x = null, double? y = null)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_show_context_menu(
                handle, menu.ToJson(), x ?? -1, y ?? -1);
        return this;
    }

    // --- Menu items (post-run) ---
    // Changes apply to every item with this id in the menu bar and in the tray menu. Context menus
    // are built from their RustinoMenu at every ShowContextMenu.

    public RustinoWindow SetMenuItemEnabled(string id, bool enabled)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu_item_enabled(handle, id, enabled ? 1 : 0);
        return this;
    }

    public RustinoWindow SetMenuItemChecked(string id, bool isChecked)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu_item_checked(handle, id, isChecked ? 1 : 0);
        return this;
    }

    public RustinoWindow SetMenuItemText(string id, string text)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(text);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu_item_text(handle, id, text);
        return this;
    }

    // --- System Tray (post-run) ---

    // tooltip: also the icon's accessible name, defaulting to the window title (ignored on Linux,
    // where only title identifies the icon).
    // title: text next to the icon (macOS, Linux). isTemplateIcon (macOS): the icon's alpha is used
    // as a mask that follows the menu bar's colors. menuOnLeftClick: false leaves left clicks to
    // TrayIconClicked (macOS, Windows).
    public RustinoWindow SetTrayIcon(string iconPath, string? tooltip = null, RustinoMenu? menu = null,
        string? title = null, bool isTemplateIcon = false, bool menuOnLeftClick = true) =>
        SetTrayIcon(iconPath, tempFile: false, tooltip, menu, title, isTemplateIcon, menuOnLeftClick);

    // Written to a temporary file, deleted when the tray icon is replaced or removed and by Dispose
    public RustinoWindow SetTrayIcon(Stream icon, string? tooltip = null, RustinoMenu? menu = null,
        string? title = null, bool isTemplateIcon = false, bool menuOnLeftClick = true)
    {
        ThrowIfDisposed();
        return SetTrayIcon(WriteTempFile(icon, "rustino_tray"), tempFile: true,
            tooltip, menu, title, isTemplateIcon, menuOnLeftClick);
    }

    private RustinoWindow SetTrayIcon(string iconPath, bool tempFile, string? tooltip, RustinoMenu? menu,
        string? title, bool isTemplateIcon, bool menuOnLeftClick)
    {
        var used = false;
        try
        {
            ThrowIfDisposed();
            if (Native is { } handle)
            {
                RustinoDllImports.rustino_set_tray_icon(
                    handle, iconPath, tooltip, menu?.ToJson(), title,
                    isTemplateIcon ? 1 : 0, menuOnLeftClick ? 1 : 0);
                used = true;
            }
        }
        finally
        {
            if (tempFile && !used) DeleteFile(iconPath);
        }
        DeleteFile(Interlocked.Exchange(ref _trayIconTempFile, used && tempFile ? iconPath : null));
        return this;
    }

    // Text next to the tray icon (macOS, Linux); null removes it.
    public RustinoWindow SetTrayTitle(string? title)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_tray_title(handle, title);
        return this;
    }

    // --- Badge ---

    public RustinoWindow SetBadgeCount(int? count, string? background = null, string? foreground = null)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
        {
            var (bgR, bgG, bgB) = ParseHexColor(background, 0xE0, 0x1E, 0x5A);
            var (fgR, fgG, fgB) = ParseHexColor(foreground, 0xFF, 0xFF, 0xFF);
            RustinoDllImports.rustino_set_badge_count(handle, count ?? 0, bgR, bgG, bgB, fgR, fgG, fgB);
        }
        return this;
    }

    public RustinoWindow ClearBadge()
    {
        ThrowIfDisposed();
        return SetBadgeCount(null);
    }

    private static (byte r, byte g, byte b) ParseHexColor(string? hex, byte defaultR, byte defaultG, byte defaultB)
    {
        if (string.IsNullOrEmpty(hex))
            return (defaultR, defaultG, defaultB);
        var s = hex.StartsWith('#') ? hex[1..] : hex;
        if (s.Length == 6 &&
            byte.TryParse(s[0..2], System.Globalization.NumberStyles.HexNumber, null, out var r) &&
            byte.TryParse(s[2..4], System.Globalization.NumberStyles.HexNumber, null, out var g) &&
            byte.TryParse(s[4..6], System.Globalization.NumberStyles.HexNumber, null, out var b))
        {
            return (r, g, b);
        }
        return (defaultR, defaultG, defaultB);
    }

    public RustinoWindow RemoveTrayIcon()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_remove_tray_icon(handle);
        DeleteFile(Interlocked.Exchange(ref _trayIconTempFile, null));
        return this;
    }

    // --- Dialogs (post-run) ---
    // While the window runs, dialogs are modal for it (sheets on macOS) and can be called from any thread.

    public string[]? ShowOpenFileDialog(
        string? title = null,
        string? defaultPath = null,
        FileFilter[]? filters = null,
        bool multiSelect = false)
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        var filterStr = FileFilter.Encode(filters);
        var ptr = RustinoDllImports.rustino_show_open_file_dialog(
            handle, title, defaultPath, filterStr, multiSelect ? 1 : 0);
        return ConsumePathResult(ptr);
    }

    public string? ShowSaveFileDialog(
        string? title = null,
        string? defaultPath = null,
        FileFilter[]? filters = null)
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        var filterStr = FileFilter.Encode(filters);
        var ptr = RustinoDllImports.rustino_show_save_file_dialog(
            handle, title, defaultPath, filterStr);
        return ConsumeStringResult(ptr);
    }

    public string[]? ShowSelectFolderDialog(
        string? title = null,
        string? defaultPath = null,
        bool multiSelect = false)
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        var ptr = RustinoDllImports.rustino_show_select_folder_dialog(
            handle, title, defaultPath, multiSelect ? 1 : 0);
        return ConsumePathResult(ptr);
    }

    // Photino's ShowMessage. Before the window runs (or after it closes) the message box has no parent.
    public RustinoDialogResult ShowMessage(
        string title,
        string text,
        RustinoDialogButtons buttons = RustinoDialogButtons.Ok,
        RustinoDialogIcon icon = RustinoDialogIcon.Info)
    {
        ThrowIfDisposed();
        // The parent is optional, so it's passed as a pointer: keep the window alive by hand
        var parent = Native;
        var addedRef = false;
        try
        {
            parent?.DangerousAddRef(ref addedRef);
            return (RustinoDialogResult)RustinoDllImports.rustino_show_message(
                addedRef ? parent!.DangerousGetHandle() : IntPtr.Zero, title, text, (int)buttons, (int)icon);
        }
        finally
        {
            if (addedRef)
                parent!.DangerousRelease();
        }
    }

    private static string WriteTempFile(Stream content, string prefix)
    {
        var path = Path.Combine(Path.GetTempPath(), $"{prefix}_{Guid.NewGuid():N}.png");
        try
        {
            using var file = File.Create(path);
            content.CopyTo(file);
        }
        catch
        {
            DeleteFile(path);
            throw;
        }
        return path;
    }

    private static void DeleteFile(string? path)
    {
        if (path is null) return;
        try { File.Delete(path); }
        catch (Exception) { }
    }

    private void DeleteTempIconFiles()
    {
        DeleteFile(Interlocked.Exchange(ref _iconTempFile, null));
        DeleteFile(Interlocked.Exchange(ref _trayIconTempFile, null));
    }

    private static string? ConsumeStringResult(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return null;
        var result = Marshal.PtrToStringUTF8(ptr);
        RustinoDllImports.rustino_free_string(ptr);
        return result;
    }

    private static string[]? ConsumePathResult(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return null;
        var joined = Marshal.PtrToStringUTF8(ptr);
        RustinoDllImports.rustino_free_string(ptr);
        return joined?.Split('\n', StringSplitOptions.RemoveEmptyEntries);
    }

    // --- Blocking run ---

    /// <summary>
    /// Shows the window and blocks until it closes. The first window runs the app's event loop
    /// on the calling thread (on macOS, the main thread). Windows started while it runs join
    /// that event loop: call their WaitForClose() from another thread. Their events are raised
    /// on the first window's thread, and they close when the first window closes.
    /// </summary>
    /// <exception cref="RustinoException">The native window or its webview could not be created, or the native event loop failed.</exception>
    public void WaitForClose()
    {
        ThrowIfDisposed();

        RustinoWindowHandle nativeHandle;
        var addedRef = false;
        lock (_lifecycleLock)
        {
            ThrowIfDisposed();
            if (_runStarted)
                throw new InvalidOperationException("WaitForClose() can only start the native window once.");
            nativeHandle = EnsureNativeLocked();
            // Dispose only closes a running window: this reference keeps the native window until
            // the native call returned, then releasing it frees the window
            nativeHandle.DangerousAddRef(ref addedRef);
            RegisterCallbacks(nativeHandle);
            _runStarted = true;
        }

        try
        {
            int status;
            IntPtr errorPtr;
            if (OperatingSystem.IsWindows()
                && Thread.CurrentThread.GetApartmentState() != ApartmentState.STA)
            {
                status = 0;
                errorPtr = IntPtr.Zero;
                var thread = new Thread(() =>
                {
                    status = RustinoDllImports.rustino_wait_for_exit(nativeHandle, out var threadError);
                    errorPtr = threadError;
                });
                thread.SetApartmentState(ApartmentState.STA);
                thread.Start();
                thread.Join();
            }
            else
            {
                status = RustinoDllImports.rustino_wait_for_exit(nativeHandle, out errorPtr);
            }

            if (status != 0)
                throw new RustinoException(ConsumeStringResult(errorPtr) ?? "The native window failed without an error message.");
            if (errorPtr != IntPtr.Zero)
                RustinoDllImports.rustino_free_string(errorPtr);
        }
        finally
        {
            Instances.TryRemove(new KeyValuePair<IntPtr, RustinoWindow>(nativeHandle.Key, this));
            if (addedRef)
                nativeHandle.DangerousRelease();
        }
    }

    public void Close()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_close(handle);
    }

    // --- Native lifecycle ---

    // The native window once it exists: null before, and after Dispose. A call that races with
    // Dispose gets an ObjectDisposedException from the marshaller, never a freed window.
    private RustinoWindowHandle? Native => _nativeHandle is { IsClosed: false } handle ? handle : null;

    private RustinoWindowHandle EnsureNative()
    {
        lock (_lifecycleLock)
            return EnsureNativeLocked();
    }

    private void SetCreationOnly<T>(ref T field, T value, string setting, Func<RustinoWindowHandle, T, int> nativeSetter)
    {
        lock (_lifecycleLock)
        {
            ThrowIfDisposed();
            if (_runStarted)
                throw CreationSettingStarted(setting);
            if (Native is { } handle && nativeSetter(handle, value) == 0)
                throw CreationSettingStarted(setting);
            field = value;
        }
    }

    private static InvalidOperationException CreationSettingStarted(string setting) =>
        new($"{setting} can only be changed before WaitForClose() starts the native window.");

    private RustinoWindowHandle EnsureNativeLocked()
    {
        ThrowIfDisposed();
        if (_nativeHandle is { } existing) return existing;

        var titlePtr = Marshal.StringToCoTaskMemUTF8(_title);
        var iconPtr = _iconFile != null ? Marshal.StringToCoTaskMemUTF8(_iconFile) : IntPtr.Zero;
        var aboutNamePtr = _aboutName != null ? Marshal.StringToCoTaskMemUTF8(_aboutName) : IntPtr.Zero;
        var aboutVersionPtr = _aboutVersion != null ? Marshal.StringToCoTaskMemUTF8(_aboutVersion) : IntPtr.Zero;
        var aboutCopyrightPtr = _aboutCopyright != null ? Marshal.StringToCoTaskMemUTF8(_aboutCopyright) : IntPtr.Zero;
        var aboutWebsitePtr = _aboutWebsite != null ? Marshal.StringToCoTaskMemUTF8(_aboutWebsite) : IntPtr.Zero;
        var aboutLicensePtr = _aboutLicense != null ? Marshal.StringToCoTaskMemUTF8(_aboutLicense) : IntPtr.Zero;
        var aboutAuthorsPtr = _aboutAuthors.Count > 0 ? Marshal.StringToCoTaskMemUTF8(string.Join("\n", _aboutAuthors)) : IntPtr.Zero;
        var aboutCommentsPtr = _aboutComments != null ? Marshal.StringToCoTaskMemUTF8(_aboutComments) : IntPtr.Zero;

        // The handle roots only a WeakReference, so logging does not keep the RustinoWindow
        // alive forever. Install the callback before SetLogger too, so logging can be enabled
        // after Load() has created the native instance. The static delegate remains rooted.
        // RustinoWindowHandle frees it with the native window.
        var logContext = GCHandle.Alloc(new WeakReference<RustinoWindow>(this));
        var logCallbackPtr = Marshal.GetFunctionPointerForDelegate(LogCb);
        var logContextPtr = GCHandle.ToIntPtr(logContext);
        RustinoWindowHandle handle;

        try
        {
            var parameters = new RustinoNativeParameters
            {
                Title = titlePtr,
                IconFile = iconPtr,
                Width = _width,
                Height = _height,
                CenterOnInitialize = _center ? 1 : 0,
                UseOsDefaultSize = _useOsDefaultSize ? 1 : 0,
                Resizable = _resizable ? 1 : 0,
                Topmost = _topmost ? 1 : 0,
                DevToolsEnabled = _devToolsEnabled ? 1 : 0,
                ClipboardEnabled = _clipboardEnabled ? 1 : 0,
                IgnoreCertificateErrors = _ignoreCertErrors ? 1 : 0,
                WebSecurityEnabled = _webSecurityEnabled ? 1 : 0,
                LogVerbosity = _logVerbosity,
                LogCallback = logCallbackPtr,
                LogContext = logContextPtr,
                AboutName = aboutNamePtr,
                AboutVersion = aboutVersionPtr,
                AboutCopyright = aboutCopyrightPtr,
                AboutWebsite = aboutWebsitePtr,
                AboutLicense = aboutLicensePtr,
                AboutAuthors = aboutAuthorsPtr,
                AboutComments = aboutCommentsPtr,
            };

            var instance = RustinoDllImports.rustino_ctor(ref parameters);
            if (instance == IntPtr.Zero)
                throw new InvalidOperationException("Failed to create native Rustino window.");
            handle = new RustinoWindowHandle(instance, logContext);
        }
        catch
        {
            logContext.Free();
            throw;
        }
        finally
        {
            Marshal.FreeCoTaskMem(titlePtr);
            if (iconPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(iconPtr);
            if (aboutNamePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(aboutNamePtr);
            if (aboutVersionPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(aboutVersionPtr);
            if (aboutCopyrightPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(aboutCopyrightPtr);
            if (aboutWebsitePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(aboutWebsitePtr);
            if (aboutLicensePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(aboutLicensePtr);
            if (aboutAuthorsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(aboutAuthorsPtr);
            if (aboutCommentsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(aboutCommentsPtr);
        }

        try
        {
            // Apply extended configuration via setters
            if (_transparent)
                RustinoDllImports.rustino_set_transparent(handle, 1);
            if (!_decorations)
                RustinoDllImports.rustino_set_decorations(handle, 0);
            ApplyExtConfiguration(handle);
            ApplyWebViewConfiguration(handle);
            if (!_visible)
                RustinoDllImports.rustino_set_visible(handle, 0);
            if (_maximized)
                RustinoDllImports.rustino_set_maximized(handle, 1);
            if (_fullscreen)
                RustinoDllImports.rustino_set_fullscreen(handle, 1);
            if (_position is { } pos)
                RustinoDllImports.rustino_set_position(handle, pos.X, pos.Y);
            if (_minSize is { } min)
                RustinoDllImports.rustino_set_min_size(handle, min.Width, min.Height);
            if (_maxSize is { } max)
                RustinoDllImports.rustino_set_max_size(handle, max.Width, max.Height);
            if (_backgroundColor is { } bg)
                RustinoDllImports.rustino_set_background_color(handle, bg.R, bg.G, bg.B, bg.A);
            if (_userAgent != null)
                RustinoDllImports.rustino_set_user_agent(handle, _userAgent);
            if (_userDataFolder != null)
                RustinoDllImports.rustino_set_user_data_folder(handle, _userDataFolder);
            if (!_mediaAutoplay)
                RustinoDllImports.rustino_set_media_autoplay(handle, 0);
            if (_zoomHotkeys)
                RustinoDllImports.rustino_set_zoom_hotkeys(handle, 1);
            foreach (var script in _initScripts)
                RustinoDllImports.rustino_add_init_script(handle, script);
            foreach (var scheme in _customSchemes.Keys)
                RustinoDllImports.rustino_add_custom_scheme(handle, scheme);

            if (_iconFile != null && OperatingSystem.IsMacOS())
            {
                SetMacDockIcon(_iconFile);
            }
        }
        catch
        {
            handle.Dispose();
            throw;
        }
        _nativeHandle = handle;
        return handle;
    }

    // --- Callback wiring ---

    private void RegisterCallbacks(RustinoWindowHandle handle)
    {
        Instances[handle.Key] = this;
        RustinoDllImports.rustino_set_callback_context(handle, handle.Key);
        RustinoDllImports.rustino_set_closing_handler(handle, ClosingCb);
        RustinoDllImports.rustino_set_closed_handler(handle, ClosedCb);
        RustinoDllImports.rustino_set_resized_handler(handle, ResizedCb);
        RustinoDllImports.rustino_set_moved_handler(handle, MovedCb);
        RustinoDllImports.rustino_set_focus_changed_handler(handle, FocusCb);
        RegisterExtCallbacks(handle);
        RegisterWebViewCallbacks(handle);
        RustinoDllImports.rustino_set_web_message_received_handler(handle, WebMsgCb);
        RustinoDllImports.rustino_set_page_load_handler(handle, PageLoadCb);
        RustinoDllImports.rustino_set_navigation_handler(handle, NavCb);
        RustinoDllImports.rustino_set_menu_event_handler(handle, MenuItemCb);
        RustinoDllImports.rustino_set_tray_icon_event_handler(handle, TrayCb);
        RustinoDllImports.rustino_set_custom_scheme_handler(handle, CustomSchemeCb);
    }


    // --- Static native callbacks ---

    private static int OnClosingNative(IntPtr ctx) => GuardNativeCallback(ctx, nameof(OnClosingNative), 0, w =>
    {
        if (w.WindowClosing is not { } handler) return 0;
        var args = new CancelEventArgs();
        handler.Invoke(w, args);
        return args.Cancel ? 1 : 0;
    });

    // A failing handler or observer doesn't keep the streams from completing
    private static void OnClosedNative(IntPtr ctx) => GuardNativeCallback(ctx, nameof(OnClosedNative), w =>
    {
        try { w.WindowClosed?.Invoke(w, EventArgs.Empty); }
        catch (Exception exception) { ReportCallbackException(w, nameof(OnClosedNative), exception); }
        try { w._windowClosedObs.Emit(EventArgs.Empty); }
        catch (Exception exception) { ReportCallbackException(w, nameof(OnClosedNative), exception); }
        CompleteAllObservables(w);
    });

    private static void OnResizedNative(IntPtr ctx, int width, int height) => GuardNativeCallback(ctx, nameof(OnResizedNative), w =>
    {
        w.SizeChanged?.Invoke(w, new SizeEventArgs(width, height));
        w._sizeChangedObs.Emit((width, height));
    });

    private static void OnMovedNative(IntPtr ctx, int x, int y) => GuardNativeCallback(ctx, nameof(OnMovedNative), w =>
    {
        w.LocationChanged?.Invoke(w, new PointEventArgs(x, y));
        w._locationChangedObs.Emit((x, y));
    });

    private static void OnFocusChangedNative(IntPtr ctx, int focused) => GuardNativeCallback(ctx, nameof(OnFocusChangedNative), w =>
    {
        var isFocused = focused != 0;
        w.FocusChanged?.Invoke(w, isFocused);
        w._focusChangedObs.Emit(isFocused);
    });

    private static void OnWebMessageNative(IntPtr ctx, IntPtr msgPtr, IntPtr sourceUrlPtr) => GuardNativeCallback(ctx, nameof(OnWebMessageNative), w =>
    {
        var msg = Marshal.PtrToStringUTF8(msgPtr);
        if (msg == null) return;
        w.WebMessageReceived?.Invoke(w, msg);
        w._webMessageObs.Emit(msg);

        var args = new WebMessageEventArgs(msg, Marshal.PtrToStringUTF8(sourceUrlPtr) ?? "");
        w.WebMessageReceivedWithSource?.Invoke(w, args);
        w._webMessageWithSourceObs.Emit(args);
    });

    private static void OnPageLoadNative(IntPtr ctx, int eventType, IntPtr urlPtr) => GuardNativeCallback(ctx, nameof(OnPageLoadNative), w =>
    {
        var url = Marshal.PtrToStringUTF8(urlPtr) ?? "";
        var args = new PageLoadEventArgs(eventType == 0, url);
        w.PageLoaded?.Invoke(w, args);
        w._pageLoadedObs.Emit(args);

        if (OperatingSystem.IsMacOS() && eventType == 1 && w._iconFile != null)
        {
            SetMacDockIcon(w._iconFile);
        }
    });

    private static int OnNavigationNative(IntPtr ctx, IntPtr urlPtr) => GuardNativeCallback(ctx, nameof(OnNavigationNative), 0, w =>
    {
        var url = Marshal.PtrToStringUTF8(urlPtr) ?? "";
        var args = new NavigationEventArgs(url);
        w.Navigating?.Invoke(w, args);
        w._navigatingObs.Emit(args);
        return args.Cancel ? 1 : 0;
    });

    private static void OnMenuItemClickedNative(IntPtr ctx, IntPtr idPtr, int isChecked) => GuardNativeCallback(ctx, nameof(OnMenuItemClickedNative), w =>
    {
        var id = Marshal.PtrToStringUTF8(idPtr);
        if (id == null) return;
        w.MenuItemClicked?.Invoke(w, id);
        w._menuItemClickedObs.Emit(id);

        if (isChecked < 0) return;
        var args = new MenuItemCheckedEventArgs(id, isChecked != 0);
        w.MenuItemCheckedChanged?.Invoke(w, args);
        w._menuItemCheckedChangedObs.Emit(args);
    });

    private static void OnTrayIconClickedNative(IntPtr ctx, int button, int x, int y) => GuardNativeCallback(ctx, nameof(OnTrayIconClickedNative), w =>
    {
        var args = new TrayIconClickedEventArgs((TrayMouseButton)button, x, y);
        w.TrayIconClicked?.Invoke(w, args);
        w._trayIconClickedObs.Emit(args);
    });

    private static void OnCustomSchemeNative(IntPtr ctx, IntPtr urlPtr, IntPtr response) => GuardNativeCallback(ctx, nameof(OnCustomSchemeNative), w =>
    {
        var url = Marshal.PtrToStringUTF8(urlPtr);
        if (url == null) return;
        var scheme = url.Split(':', 2)[0];
        if (!w._customSchemes.TryGetValue(scheme, out var handler)) return;

        using var content = handler(w, scheme, url, out var contentType);
        if (content == null) return;
        using var buffer = new MemoryStream();
        content.CopyTo(buffer);
        RustinoDllImports.rustino_set_scheme_response(response, buffer.GetBuffer(), (int)buffer.Length, contentType);
    });

    private static void OnLogMessageNative(IntPtr ctx, int level, IntPtr messagePtr)
    {
        RustinoWindow? window = null;
        try
        {
            if (ctx == IntPtr.Zero) return;
            var handle = GCHandle.FromIntPtr(ctx);
            if (handle.Target is not WeakReference<RustinoWindow> weak || !weak.TryGetTarget(out window)) return;
            if (window._logger == null) return;

            var message = Marshal.PtrToStringUTF8(messagePtr);
            if (string.IsNullOrEmpty(message)) return;

            var logLevel = level switch
            {
                0 => LogLevel.Trace,
                1 => LogLevel.Debug,
                2 => LogLevel.Information,
                3 => LogLevel.Warning,
                4 => LogLevel.Error,
                5 => LogLevel.Critical,
                _ => LogLevel.Information
            };

            window._logger.Log(logLevel, message);
        }
        catch (Exception exception)
        {
            if (window is not null)
                ReportCallbackException(window, nameof(OnLogMessageNative), exception);
        }
    }

    private static void GuardNativeCallback(IntPtr context, string callbackName, Action<RustinoWindow> callback)
    {
        GuardNativeCallback(context, callbackName, 0, window =>
        {
            callback(window);
            return 0;
        });
    }

    private static T GuardNativeCallback<T>(IntPtr context, string callbackName, T fallback, Func<RustinoWindow, T> callback)
    {
        RustinoWindow? window = null;
        try
        {
            if (!Instances.TryGetValue(context, out window)) return fallback;
            return callback(window);
        }
        catch (Exception exception)
        {
            if (window is not null)
                ReportCallbackException(window, callbackName, exception);
            return fallback;
        }
    }

    private static void ReportCallbackException(RustinoWindow window, string callbackName, Exception exception)
    {
        try { window._logger?.LogError(exception, "Unhandled exception in Rustino callback {CallbackName}.", callbackName); }
        catch (Exception) { }

        try { window.UnhandledCallbackException?.Invoke(window, new NativeCallbackExceptionEventArgs(callbackName, exception)); }
        catch (Exception reportingException)
        {
            try { window._logger?.LogError(reportingException, "Unhandled exception in Rustino callback exception handler."); }
            catch (Exception) { }
        }
    }

    // --- Observable completion ---

    // Every stream completes even when observers throw; their exceptions are reported
    private static void CompleteAllObservables(RustinoWindow w)
    {
        ICompletable[] observables =
        [
            w._sizeChangedObs,
            .. w.ExtObservables,
            .. w.WebViewObservables,
            w._locationChangedObs,
            w._focusChangedObs,
            w._webMessageObs,
            w._webMessageWithSourceObs,
            w._pageLoadedObs,
            w._navigatingObs,
            w._windowClosedObs,
            w._menuItemClickedObs,
            w._menuItemCheckedChangedObs,
            w._trayIconClickedObs,
        ];
        foreach (var observable in observables)
        {
            try { observable.Complete(); }
            catch (Exception exception) { ReportCallbackException(w, nameof(IObserver<object>.OnCompleted), exception); }
        }
    }

    // --- Dispose ---

    // The native window is freed once no call uses it anymore: at once, or, while the window runs,
    // when it has closed and WaitForClose returns. Without Dispose, RustinoWindowHandle's finalizer frees it.
    public void Dispose()
    {
        if (Interlocked.CompareExchange(ref _disposed, 1, 0) != 0)
            return;

        try
        {
            CompleteAllObservables(this);
        }
        finally
        {
            RustinoWindowHandle? handle;
            bool runStarted;
            lock (_lifecycleLock)
            {
                handle = _nativeHandle;
                runStarted = _runStarted;
            }
            if (handle is not null)
            {
                Instances.TryRemove(new KeyValuePair<IntPtr, RustinoWindow>(handle.Key, this));
                // A running window only closes here: WaitForClose holds a reference until the native
                // call returned, then the native window is freed. Not under the lock: the close may
                // wait for the event loop, whose handlers may need the lock (e.g. Load).
                if (runStarted)
                    RustinoDllImports.rustino_close(handle);
                handle.Dispose();
            }
            DeleteTempIconFiles();
        }
    }

    private void ThrowIfDisposed() => ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) != 0, this);

    // --- dynamic macOS Dock Icon / Windows AppId Helpers ---

    [DllImport("/usr/lib/libobjc.A.dylib")]
    private static extern IntPtr objc_getClass(string name);

    [DllImport("/usr/lib/libobjc.A.dylib")]
    private static extern IntPtr sel_registerName(string name);

    [DllImport("/usr/lib/libobjc.A.dylib", EntryPoint = "objc_msgSend")]
    private static extern IntPtr objc_msgSend(IntPtr receiver, IntPtr selector);

    [DllImport("/usr/lib/libobjc.A.dylib", EntryPoint = "objc_msgSend")]
    private static extern IntPtr objc_msgSend(IntPtr receiver, IntPtr selector, IntPtr arg);

    [DllImport("shell32.dll", SetLastError = true)]
    private static extern void SetCurrentProcessExplicitAppUserModelID([MarshalAs(UnmanagedType.LPWStr)] string appId);

    private string? _applicationId;

    public RustinoWindow SetApplicationId(string applicationId)
    {
        ThrowIfDisposed();
        _applicationId = applicationId;
        if (OperatingSystem.IsWindows() && !IsDotnetTool())
        {
            try
            {
                SetCurrentProcessExplicitAppUserModelID(applicationId);
            }
            catch { }
        }
        return this;
    }

    private static IntPtr CreateNSString(string str)
    {
        IntPtr nsStringClass = objc_getClass("NSString");
        IntPtr stringWithUTF8StringSel = sel_registerName("stringWithUTF8String:");
        IntPtr utf8Ptr = Marshal.StringToCoTaskMemUTF8(str);
        try
        {
            return objc_msgSend(nsStringClass, stringWithUTF8StringSel, utf8Ptr);
        }
        finally
        {
            Marshal.FreeCoTaskMem(utf8Ptr);
        }
    }

    [DllImport("libSystem.dylib")]
    private static extern IntPtr dlopen(string path, int mode);

    private static void SetMacDockIcon(string iconPath)
    {
        try
        {
            if (OperatingSystem.IsMacOS())
            {
                dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", 1);
            }

            IntPtr nsImageClass = objc_getClass("NSImage");
            if (nsImageClass == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to load NSImage class in SetMacDockIcon.");
                return;
            }

            IntPtr nsStringPath = CreateNSString(iconPath);
            if (nsStringPath == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to create NSString for path in SetMacDockIcon.");
                return;
            }

            IntPtr allocSel = sel_registerName("alloc");
            IntPtr nsImageAllocated = objc_msgSend(nsImageClass, allocSel);
            if (nsImageAllocated == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to allocate NSImage in SetMacDockIcon.");
                return;
            }

            IntPtr initSel = sel_registerName("initWithContentsOfFile:");
            IntPtr nsImage = objc_msgSend(nsImageAllocated, initSel, nsStringPath);
            if (nsImage == IntPtr.Zero)
            {
                Console.Error.WriteLine($"Failed to init NSImage from path in SetMacDockIcon: {iconPath}");
                return;
            }

            IntPtr nsAppClass = objc_getClass("NSApplication");
            if (nsAppClass == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to load NSApplication class in SetMacDockIcon.");
                return;
            }

            IntPtr sharedAppSel = sel_registerName("sharedApplication");
            IntPtr nsApp = objc_msgSend(nsAppClass, sharedAppSel);
            if (nsApp == IntPtr.Zero)
            {
                Console.Error.WriteLine("Failed to get NSApplication instance in SetMacDockIcon.");
                return;
            }

            IntPtr setIconSel = sel_registerName("setApplicationIconImage:");
            objc_msgSend(nsApp, setIconSel, nsImage);

            IntPtr dockTileSel = sel_registerName("dockTile");
            IntPtr dockTile = objc_msgSend(nsApp, dockTileSel);
            if (dockTile != IntPtr.Zero)
            {
                IntPtr displaySel = sel_registerName("display");
                objc_msgSend(dockTile, displaySel);
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Failed to set macOS Dock icon: {ex}");
        }
    }

    private static bool IsDotnetTool()
    {
        var processPath = Environment.ProcessPath ?? string.Empty;
        if (processPath.Contains(".dotnet") && (processPath.Contains("tools") || processPath.Contains("store")))
            return true;

        var argv0 = Environment.GetCommandLineArgs().FirstOrDefault() ?? string.Empty;
        if (argv0.Contains(".dotnet") && (argv0.Contains("tools") || argv0.Contains("store")))
            return true;

        return false;
    }
}
