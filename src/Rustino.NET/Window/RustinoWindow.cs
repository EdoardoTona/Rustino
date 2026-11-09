using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Extensions.Logging;

namespace Rustino.NET;

// A native window with a webview. The class is split by area:
// - Configuration: the settings, before and while the window runs
// - State, Menus, Dialogs, WebView, Ext, Desktop, ...: what a window can do
// - Callbacks: the events coming from the native window
// This file holds the state, the events and the lifecycle (WaitForClose, Dispose).
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

        StartMonitorsPolling();
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
            StopMonitorsPolling();
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
            if ((_userDataFolder ?? DefaultUserDataFolder()) is { } userDataFolder)
                RustinoDllImports.rustino_set_user_data_folder(handle, userDataFolder);
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

    // --- Temporary files and native strings ---

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
}
