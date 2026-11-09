using System.Text.Json;

namespace Rustino.NET;

// State of the running window, its webview and the monitors.
public partial class RustinoWindow
{
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
}
