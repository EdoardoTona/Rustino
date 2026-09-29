using System.Runtime.InteropServices;

namespace Rustino.NET;

// Native window features: theme, taskbar progress, attention requests, drag regions,
// macOS title bar, less common window flags and macOS app events.
public partial class RustinoWindow
{
    private WindowTheme _theme = WindowTheme.System;
    private bool? _shadow;
    private bool _skipTaskbar;
    private bool _contentProtection;
    private bool _visibleOnAllWorkspaces;
    private bool _closable = true;
    private bool _minimizable = true;
    private bool _maximizable = true;
    private bool _alwaysOnBottom;
    private bool _ignoreCursorEvents;
    private MacTitleBarStyle _macTitleBarStyle;
    private (double X, double Y)? _trafficLightPosition;
    private string? _desktopFileName;
    private bool _dragRegions = true;

    private readonly EventObservable<WindowTheme> _themeChangedObs = new();
    private readonly EventObservable<ScaleFactorChangedEventArgs> _scaleFactorChangedObs = new();
    private readonly EventObservable<string[]> _urlsOpenedObs = new();
    private readonly EventObservable<bool> _reopenedObs = new();

    private static readonly IntCallback ThemeChangedCb = OnThemeChangedNative;
    private static readonly ScaleFactorCallback ScaleFactorChangedCb = OnScaleFactorChangedNative;
    private static readonly StringCallback UrlsOpenedCb = OnUrlsOpenedNative;
    private static readonly IntCallback ReopenCb = OnReopenNative;

    /// <summary>The window theme changed (system change or <see cref="SetTheme"/>): Light or Dark.</summary>
    public event EventHandler<WindowTheme>? ThemeChanged;

    /// <summary>The window moved to a monitor with another scale factor, or the user changed it.</summary>
    public event EventHandler<ScaleFactorChangedEventArgs>? ScaleFactorChanged;

    /// <summary>
    /// macOS: the app was asked to open files or URLs (file associations and URL schemes declared in the
    /// app bundle's Info.plist).
    /// </summary>
    public event EventHandler<string[]>? UrlsOpened;

    /// <summary>
    /// macOS: the user clicked the Dock icon of the running app. The argument tells whether a window is
    /// visible: when it isn't (e.g. hidden to the tray), show it again.
    /// </summary>
    public event EventHandler<bool>? Reopened;

    public IObservable<WindowTheme> WhenThemeChanged => _themeChangedObs;
    public IObservable<ScaleFactorChangedEventArgs> WhenScaleFactorChanged => _scaleFactorChangedObs;
    public IObservable<string[]> WhenUrlsOpened => _urlsOpenedObs;
    public IObservable<bool> WhenReopened => _reopenedObs;

    /// <summary>
    /// Light, dark or system theme for the title bar, the native controls and <c>prefers-color-scheme</c>.
    /// Windows applies it to this window; macOS and Linux to the whole app.
    /// </summary>
    public RustinoWindow SetTheme(WindowTheme theme)
    {
        ThrowIfDisposed();
        _theme = theme;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_theme(_nativeHandle, (int)theme);
        return this;
    }

    /// <summary>Current theme of the window, Light or Dark (System before the window runs).</summary>
    public WindowTheme Theme =>
        _nativeHandle == IntPtr.Zero ? WindowTheme.System : (WindowTheme)RustinoExtDllImports.rustino_get_theme(_nativeHandle);

    /// <summary>Ratio between physical and logical pixels of the window's monitor.</summary>
    public double ScaleFactor =>
        _nativeHandle == IntPtr.Zero ? 1.0 : RustinoExtDllImports.rustino_get_scale_factor(_nativeHandle);

    /// <summary>
    /// Progress on the taskbar button (Windows), the Dock icon (macOS) or the dock icon (Linux, through the
    /// Unity LauncherEntry API: see <see cref="SetDesktopFileName"/>). On macOS and Linux the progress is the app's.
    /// </summary>
    /// <param name="progress">0-100; null keeps the current value.</param>
    public RustinoWindow SetProgressBar(ProgressBarState state, int? progress = null)
    {
        ThrowIfDisposed();
        if (progress is < 0 or > 100)
            throw new ArgumentOutOfRangeException(nameof(progress), "Progress must be between 0 and 100.");
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_progress_bar(_nativeHandle, (int)state, progress ?? -1);
        return this;
    }

    public RustinoWindow ClearProgressBar() => SetProgressBar(ProgressBarState.None);

    /// <summary>
    /// Flashes the taskbar button (Windows), bounces the Dock icon (macOS) or sets the urgency hint (Linux)
    /// until the app is focused. No effect while the app is focused.
    /// </summary>
    public RustinoWindow RequestUserAttention(UserAttentionType type = UserAttentionType.Informational)
    {
        ThrowIfDisposed();
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_request_user_attention(_nativeHandle, (int)type);
        return this;
    }

    /// <summary>Stops a <see cref="RequestUserAttention"/> (not supported on macOS).</summary>
    public RustinoWindow CancelUserAttentionRequest()
    {
        ThrowIfDisposed();
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_request_user_attention(_nativeHandle, 0);
        return this;
    }

    /// <summary>
    /// Plays the system alert sound: <c>MessageBeep</c> on Windows, <c>NSBeep</c> on macOS, the display
    /// bell on Linux. No effect before the window runs.
    /// </summary>
    public RustinoWindow Beep()
    {
        ThrowIfDisposed();
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_beep(_nativeHandle);
        return this;
    }

    /// <summary>
    /// Window shadow: on Windows it applies to chromeless windows, on macOS to all of them. Both platforms
    /// draw it by default; not supported on Linux.
    /// </summary>
    public RustinoWindow SetShadow(bool shadow)
    {
        ThrowIfDisposed();
        _shadow = shadow;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_shadow(_nativeHandle, shadow ? 1 : 0);
        return this;
    }

    /// <summary>
    /// Hides the taskbar button, for apps that live in the tray. On macOS the whole app leaves the Dock and
    /// the app switcher.
    /// </summary>
    public RustinoWindow SetSkipTaskbar(bool skip)
    {
        ThrowIfDisposed();
        _skipTaskbar = skip;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_skip_taskbar(_nativeHandle, skip ? 1 : 0);
        return this;
    }

    /// <summary>Keeps the window out of screenshots and screen recordings (Windows, macOS).</summary>
    public RustinoWindow SetContentProtection(bool enabled)
    {
        ThrowIfDisposed();
        _contentProtection = enabled;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_content_protection(_nativeHandle, enabled ? 1 : 0);
        return this;
    }

    /// <summary>Shows the window on every virtual desktop (macOS, Linux).</summary>
    public RustinoWindow SetVisibleOnAllWorkspaces(bool visible)
    {
        ThrowIfDisposed();
        _visibleOnAllWorkspaces = visible;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_visible_on_all_workspaces(_nativeHandle, visible ? 1 : 0);
        return this;
    }

    /// <summary>Enables the close button of the title bar (Linux: a request that the window manager may ignore).</summary>
    public RustinoWindow SetClosable(bool closable)
    {
        ThrowIfDisposed();
        _closable = closable;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_closable(_nativeHandle, closable ? 1 : 0);
        return this;
    }

    /// <summary>Enables the minimize button of the title bar. Not supported on Linux.</summary>
    public RustinoWindow SetMinimizable(bool minimizable)
    {
        ThrowIfDisposed();
        _minimizable = minimizable;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_minimizable(_nativeHandle, minimizable ? 1 : 0);
        return this;
    }

    /// <summary>Enables the maximize (zoom on macOS) button of the title bar. Not supported on Linux.</summary>
    public RustinoWindow SetMaximizable(bool maximizable)
    {
        ThrowIfDisposed();
        _maximizable = maximizable;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_maximizable(_nativeHandle, maximizable ? 1 : 0);
        return this;
    }

    /// <summary>
    /// Keeps the window below the other windows, e.g. for desktop widgets. It replaces <see cref="SetTopMost"/>.
    /// Linux: a request to the window manager, not supported on Wayland.
    /// </summary>
    public RustinoWindow SetAlwaysOnBottom(bool onBottom)
    {
        ThrowIfDisposed();
        _alwaysOnBottom = onBottom;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_always_on_bottom(_nativeHandle, onBottom ? 1 : 0);
        return this;
    }

    /// <summary>Lets the mouse clicks through the window to the windows below it, for overlays.</summary>
    public RustinoWindow SetIgnoreCursorEvents(bool ignore)
    {
        ThrowIfDisposed();
        _ignoreCursorEvents = ignore;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_ignore_cursor_events(_nativeHandle, ignore ? 1 : 0);
        return this;
    }

    /// <summary>
    /// macOS title bar style, also while the window runs. A chromeless window gets it back with
    /// <c>SetChromeless(false)</c>.
    /// </summary>
    public RustinoWindow SetMacTitleBarStyle(MacTitleBarStyle style)
    {
        ThrowIfDisposed();
        _macTitleBarStyle = style;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_mac_title_bar_style(_nativeHandle, (int)style);
        return this;
    }

    /// <summary>
    /// Elements with <c>data-rustino-drag-region</c> move the window, and on Windows and Linux the edges of the
    /// page resize chromeless windows (on by default). Any loaded page can use them: turn them off when the window
    /// shows untrusted pages. Must be called before <see cref="WaitForClose"/>.
    /// </summary>
    public RustinoWindow SetDragRegionsEnabled(bool enabled)
    {
        SetCreationOnly(ref _dragRegions, enabled, nameof(SetDragRegionsEnabled),
            static (instance, value) => RustinoExtDllImports.rustino_set_drag_regions_enabled(instance, value ? 1 : 0));
        return this;
    }

    /// <summary>Position of the macOS traffic lights, in logical pixels from the top-left corner.</summary>
    public RustinoWindow SetMacTrafficLightPosition(double x, double y)
    {
        ThrowIfDisposed();
        _trafficLightPosition = (x, y);
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_traffic_light_position(_nativeHandle, x, y);
        return this;
    }

    /// <summary>
    /// Linux: the .desktop file of the app (e.g. "com.example.App.desktop"), whose dock icon shows the badge
    /// and the progress. By default "&lt;executable name&gt;.desktop".
    /// </summary>
    public RustinoWindow SetDesktopFileName(string fileName)
    {
        ThrowIfDisposed();
        _desktopFileName = fileName;
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_set_desktop_file_name(_nativeHandle, fileName);
        return this;
    }

    /// <summary>
    /// Moves the window with the mouse; call it while the left button is down. Pages don't need it: the
    /// elements with <c>data-rustino-drag-region</c> already drag the window.
    /// </summary>
    public RustinoWindow DragWindow()
    {
        ThrowIfDisposed();
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_drag_window(_nativeHandle);
        return this;
    }

    /// <summary>Resizes the window with the mouse from an edge; call it while the left button is down.</summary>
    public RustinoWindow DragResizeWindow(ResizeDirection direction)
    {
        ThrowIfDisposed();
        if (_nativeHandle != IntPtr.Zero)
            RustinoExtDllImports.rustino_drag_resize_window(_nativeHandle, (int)direction);
        return this;
    }

    // Called by EnsureNative: settings made before the native window existed
    private void ApplyExtConfiguration()
    {
        if (_theme != WindowTheme.System)
            RustinoExtDllImports.rustino_set_theme(_nativeHandle, (int)_theme);
        if (_shadow is { } shadow)
            RustinoExtDllImports.rustino_set_shadow(_nativeHandle, shadow ? 1 : 0);
        if (_skipTaskbar)
            RustinoExtDllImports.rustino_set_skip_taskbar(_nativeHandle, 1);
        if (_contentProtection)
            RustinoExtDllImports.rustino_set_content_protection(_nativeHandle, 1);
        if (_visibleOnAllWorkspaces)
            RustinoExtDllImports.rustino_set_visible_on_all_workspaces(_nativeHandle, 1);
        if (!_closable)
            RustinoExtDllImports.rustino_set_closable(_nativeHandle, 0);
        if (!_minimizable)
            RustinoExtDllImports.rustino_set_minimizable(_nativeHandle, 0);
        if (!_maximizable)
            RustinoExtDllImports.rustino_set_maximizable(_nativeHandle, 0);
        if (_alwaysOnBottom)
            RustinoExtDllImports.rustino_set_always_on_bottom(_nativeHandle, 1);
        if (_ignoreCursorEvents)
            RustinoExtDllImports.rustino_set_ignore_cursor_events(_nativeHandle, 1);
        if (_macTitleBarStyle != MacTitleBarStyle.Default)
            RustinoExtDllImports.rustino_set_mac_title_bar_style(_nativeHandle, (int)_macTitleBarStyle);
        if (_trafficLightPosition is { } lights)
            RustinoExtDllImports.rustino_set_traffic_light_position(_nativeHandle, lights.X, lights.Y);
        if (_desktopFileName != null)
            RustinoExtDllImports.rustino_set_desktop_file_name(_nativeHandle, _desktopFileName);
        if (!_dragRegions)
            RustinoExtDllImports.rustino_set_drag_regions_enabled(_nativeHandle, 0);
    }

    // Called by RegisterCallbacks
    private void RegisterExtCallbacks()
    {
        RustinoExtDllImports.rustino_set_theme_changed_handler(_nativeHandle, ThemeChangedCb);
        RustinoExtDllImports.rustino_set_scale_factor_changed_handler(_nativeHandle, ScaleFactorChangedCb);
        RustinoExtDllImports.rustino_set_urls_opened_handler(_nativeHandle, UrlsOpenedCb);
        RustinoExtDllImports.rustino_set_reopen_handler(_nativeHandle, ReopenCb);
    }

    // Called by CompleteAllObservables
    private static void CompleteExtObservables(RustinoWindow w)
    {
        w._themeChangedObs.Complete();
        w._scaleFactorChangedObs.Complete();
        w._urlsOpenedObs.Complete();
        w._reopenedObs.Complete();
    }

    private static void OnThemeChangedNative(IntPtr ctx, int theme) => GuardNativeCallback(ctx, nameof(OnThemeChangedNative), w =>
    {
        var value = (WindowTheme)theme;
        w.ThemeChanged?.Invoke(w, value);
        w._themeChangedObs.Emit(value);
    });

    private static void OnScaleFactorChangedNative(IntPtr ctx, double scaleFactor, int width, int height) => GuardNativeCallback(ctx, nameof(OnScaleFactorChangedNative), w =>
    {
        var args = new ScaleFactorChangedEventArgs(scaleFactor, width, height);
        w.ScaleFactorChanged?.Invoke(w, args);
        w._scaleFactorChangedObs.Emit(args);
    });

    private static void OnUrlsOpenedNative(IntPtr ctx, IntPtr urlsPtr) => GuardNativeCallback(ctx, nameof(OnUrlsOpenedNative), w =>
    {
        var urls = Marshal.PtrToStringUTF8(urlsPtr)?.Split('\n', StringSplitOptions.RemoveEmptyEntries);
        if (urls is not { Length: > 0 }) return;
        urls = w.FilterUrlsOpened(urls);
        if (urls.Length == 0) return;
        w.UrlsOpened?.Invoke(w, urls);
        w._urlsOpenedObs.Emit(urls);
    });

    private static void OnReopenNative(IntPtr ctx, int hasVisibleWindows) => GuardNativeCallback(ctx, nameof(OnReopenNative), w =>
    {
        var visible = hasVisibleWindows != 0;
        w.Reopened?.Invoke(w, visible);
        w._reopenedObs.Emit(visible);
    });
}
