using System.Runtime.InteropServices;
using System.Text.Json;

namespace Rustino.NET;

// Global shortcuts and screen: cursor position, monitors added, removed or changed.
public partial class RustinoWindow
{
    // How often the monitors are compared while someone listens: tao doesn't report their changes
    private static readonly TimeSpan MonitorsPollInterval = TimeSpan.FromSeconds(1);

    private readonly EventObservable<string> _globalShortcutPressedObs = new();
    private readonly EventObservable<MonitorsChangedEventArgs> _monitorsChangedObs = new();
    private Timer? _monitorsTimer;
    // Touched on the event loop thread only
    private MonitorInfo[]? _lastMonitors;

    private static readonly StringCallback GlobalShortcutCb = OnGlobalShortcutNative;
    private static readonly StringCallback MonitorsCb = OnMonitorsNative;

    /// <summary>
    /// A global shortcut of this window was pressed, also while another app has the focus. The argument is the
    /// accelerator as it was registered.
    /// </summary>
    public event EventHandler<string>? GlobalShortcutPressed;

    /// <summary>A monitor was added, removed, or changed its bounds, scale factor or primary status.</summary>
    public event EventHandler<MonitorsChangedEventArgs>? MonitorsChanged;

    public IObservable<string> WhenGlobalShortcutPressed => _globalShortcutPressedObs;
    public IObservable<MonitorsChangedEventArgs> WhenMonitorsChanged => _monitorsChangedObs;

    /// <summary>
    /// Registers a global shortcut, such as <c>CmdOrCtrl+Shift+K</c> (the syntax of the menu accelerators), which
    /// raises <see cref="GlobalShortcutPressed"/> even while another app has the focus, until it's unregistered or
    /// the window closes. Returns false when the accelerator is invalid or another window or app has it. Before the
    /// window runs it's registered when the window starts, and only an invalid accelerator returns false.
    /// Linux: X11 only.
    /// </summary>
    public bool RegisterGlobalShortcut(string accelerator)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(accelerator);
        return RustinoExtDllImports.rustino_register_global_shortcut(EnsureNative(), accelerator) != 0;
    }

    /// <summary>Returns false when the window hadn't registered the shortcut.</summary>
    public bool UnregisterGlobalShortcut(string accelerator)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(accelerator);
        return RustinoExtDllImports.rustino_unregister_global_shortcut(EnsureNative(), accelerator) != 0;
    }

    public RustinoWindow UnregisterAllGlobalShortcuts()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoExtDllImports.rustino_unregister_all_global_shortcuts(handle);
        return this;
    }

    /// <summary>Whether the running window registered the shortcut (false before it runs).</summary>
    public bool IsGlobalShortcutRegistered(string accelerator)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(accelerator);
        return Native is { } handle && RustinoExtDllImports.rustino_is_global_shortcut_registered(handle, accelerator) != 0;
    }

    /// <summary>
    /// Position of the mouse cursor on the screen, in physical pixels; null before the window runs.
    /// Linux: always 0, 0 on Wayland.
    /// </summary>
    public (int X, int Y)? GetCursorPosition()
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        return RustinoExtDllImports.rustino_get_cursor_position(handle, out var x, out var y) != 0 ? (x, y) : null;
    }

    // Called by WaitForClose while the window runs
    private void StartMonitorsPolling() =>
        _monitorsTimer = new Timer(_ => CheckMonitors(), null, MonitorsPollInterval, MonitorsPollInterval);

    private void StopMonitorsPolling() => _monitorsTimer?.Dispose();

    private void CheckMonitors()
    {
        if (MonitorsChanged is null && !_monitorsChangedObs.HasObservers) return;
        try
        {
            if (Native is { } handle)
                RustinoExtDllImports.rustino_check_monitors(handle);
        }
        catch (ObjectDisposedException) { }
    }

    private static void OnGlobalShortcutNative(IntPtr ctx, IntPtr acceleratorPtr) => GuardNativeCallback(ctx, nameof(OnGlobalShortcutNative), w =>
    {
        if (Marshal.PtrToStringUTF8(acceleratorPtr) is not { } accelerator) return;
        w.GlobalShortcutPressed?.Invoke(w, accelerator);
        w._globalShortcutPressedObs.Emit(accelerator);
    });

    private static void OnMonitorsNative(IntPtr ctx, IntPtr json) => GuardNativeCallback(ctx, nameof(OnMonitorsNative), w =>
    {
        if (Marshal.PtrToStringUTF8(json) is not { Length: > 0 } text) return;
        var monitors = JsonSerializer.Deserialize(text, MonitorJsonContext.Default.MonitorInfoArray) ?? [];
        var previous = w._lastMonitors;
        w._lastMonitors = monitors;
        // The first check only takes the monitors to compare to
        if (previous is null || MonitorsChangedEventArgs.Compare(previous, monitors) is not { } args) return;
        w.MonitorsChanged?.Invoke(w, args);
        w._monitorsChangedObs.Emit(args);
    });
}
