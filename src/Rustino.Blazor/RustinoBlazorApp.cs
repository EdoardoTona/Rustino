using Microsoft.AspNetCore.Components;
using Microsoft.AspNetCore.Components.Web;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.FileProviders;
using Microsoft.Extensions.Logging;
using Rustino.NET;

namespace Rustino.Blazor;

/// <summary>
/// A Blazor app in a Rustino window. <see cref="Run"/> disposes the app when the main window closes:
/// the components and the scoped services of every window, then the app's services. An app that never
/// runs is disposed with <see cref="Dispose"/> or <see cref="DisposeAsync"/>.
/// </summary>
public class RustinoBlazorApp : IDisposable, IAsyncDisposable
{
    private readonly RustinoWebViewManager _manager;
    private readonly ILogger<RustinoBlazorApp>? _logger;
    private readonly object _windowsLock = new();
    private readonly Dispatcher _dispatcher;
    // The windows opened with OpenWindow, until they have been disposed
    private readonly List<(RustinoWindow Window, Task Disposed)> _openWindows = new();
    private bool _running;
    private Task? _disposal;
    // The service provider disposes the app too, as a singleton: within the disposal it has nothing to wait for
    private static readonly AsyncLocal<RustinoBlazorApp?> Disposing = new();

    internal RustinoBlazorApp(IServiceProvider services, RootComponentList rootComponents)
    {
        Services = services;
        _logger = services.GetService<ILogger<RustinoBlazorApp>>();
        MainWindow = services.GetRequiredService<RustinoWindow>()
            .SetTitle("Rustino.Blazor App")
            .SetUseOsDefaultSize(false)
            .SetSize(1000, 800)
            .Center();

        _manager = services.GetRequiredService<RustinoWebViewManager>();
        _dispatcher = services.GetRequiredService<Dispatcher>();
        foreach (var (componentType, selector) in rootComponents)
            _ = _manager.AddRootComponentAsync(componentType, selector, ParameterView.Empty);
    }

    public IServiceProvider Services { get; }

    public RustinoWindow MainWindow { get; }

    /// <summary>
    /// Raised when a window opened with <see cref="OpenWindow{TComponent}"/> fails (e.g. its webview
    /// can't be created), on the thread of that window. The window is then disposed.
    /// </summary>
    public event EventHandler<WindowFailedEventArgs>? WindowFailed;

    /// <summary>Shows the main window and blocks until it closes, then disposes the app.</summary>
    public void Run()
    {
        lock (_windowsLock)
        {
            ObjectDisposedException.ThrowIf(_disposal is not null, this);
            _running = true;
        }
        try
        {
            _manager.Navigate("/");
            MainWindow.WaitForClose();
        }
        finally
        {
            Dispose();
        }
    }

    /// <summary>
    /// Disposes the components and the scoped services of the main window, closes the windows opened with
    /// <see cref="OpenWindow{TComponent}"/> and waits until they are disposed, then disposes the app's services,
    /// including <see cref="MainWindow"/>, which closes.
    /// </summary>
    public ValueTask DisposeAsync()
    {
        if (Disposing.Value == this)
            return default;
        lock (_windowsLock)
        {
            _running = false;
            // On the thread pool: not under the lock, nor on the caller's synchronization context (e.g. the
            // renderer's dispatcher, which the disposal of the components needs)
            _disposal ??= Task.Run(DisposeCoreAsync);
            return new ValueTask(_disposal);
        }
    }

    /// <inheritdoc cref="DisposeAsync"/>
    /// <remarks>
    /// On the renderer's dispatcher (e.g. from a component) it only starts the disposal, which needs the dispatcher:
    /// await <see cref="DisposeAsync"/> there to know when it's done.
    /// </remarks>
    public void Dispose()
    {
        var disposal = DisposeAsync();
        if (!_dispatcher.CheckAccess())
            disposal.AsTask().GetAwaiter().GetResult();
    }

    private async Task DisposeCoreAsync()
    {
        try { await _manager.DisposeAsync().ConfigureAwait(false); }
        catch (Exception exception) { Log(exception, "Disposing the components of the main window failed."); }

        // They would close with the main window, which the services dispose last
        (RustinoWindow Window, Task Disposed)[] openWindows;
        lock (_windowsLock) openWindows = _openWindows.ToArray();
        foreach (var (window, _) in openWindows)
        {
            try { window.Close(); }
            catch (ObjectDisposedException) { }
        }
        await Task.WhenAll(openWindows.Select(w => w.Disposed)).ConfigureAwait(false);

        Disposing.Value = this;
        try
        {
            // The singletons: the main window, and the manager, already disposed
            if (Services is IAsyncDisposable services)
                await services.DisposeAsync().ConfigureAwait(false);
            else
                (Services as IDisposable)?.Dispose();
        }
        catch (Exception exception) { Log(exception, "Disposing the app's services failed."); }
    }

    /// <summary>
    /// Opens another window, with the same host page, rendering <typeparamref name="TComponent"/> at
    /// <paramref name="selector"/>. The window shares the app's singleton services; its components that
    /// inject <see cref="RustinoWindow"/> get the new window. Call it while the app runs (e.g. from a
    /// component): the window closes when the user closes it, with <see cref="RustinoWindow.Close"/>,
    /// or when the main window closes.
    /// </summary>
    /// <param name="parameters">Parameters of the root component.</param>
    /// <param name="configure">Configures the window before it opens (title, size, ...).</param>
    public RustinoWindow OpenWindow<TComponent>(
        string selector,
        IDictionary<string, object?>? parameters = null,
        Action<RustinoWindow>? configure = null)
        where TComponent : IComponent
    {
        if (!Volatile.Read(ref _running))
            throw OpenWindowNeedsRun();

        var window = new RustinoWindow();
        RustinoWebViewManager? manager = null;
        var started = false;
        var closed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        try
        {
            window
                .SetTitle("Rustino.Blazor App")
                .SetUseOsDefaultSize(false)
                .SetSize(800, 600)
                .Center();
            configure?.Invoke(window);

            manager = new RustinoWebViewManager(
                window,
                new RustinoWindowServices(Services, window),
                Services.GetRequiredService<Dispatcher>(),
                Services.GetRequiredService<IFileProvider>(),
                Services.GetRequiredService<JSComponentConfigurationStore>());
            _ = manager.AddRootComponentAsync(
                typeof(TComponent),
                selector,
                parameters is null ? ParameterView.Empty : ParameterView.FromDictionary(parameters));
            manager.Navigate("/");

            // The window joins the event loop of the main window: this thread only waits for its closing
            var thread = new Thread(() =>
            {
                try
                {
                    window.WaitForClose();
                }
                catch (Exception exception)
                {
                    ReportWindowFailed(window, exception, "A window opened with OpenWindow failed.");
                }
                finally
                {
                    // Nothing may escape this thread: it would end the process
                    DisposeWindowAsync(window, manager).GetAwaiter().GetResult();
                    // Added under the lock right after Start: this runs after the Add
                    lock (_windowsLock)
                        _openWindows.RemoveAll(w => ReferenceEquals(w.Window, window));
                    closed.SetResult();
                }
            })
            {
                IsBackground = true,
                Name = "Rustino.Blazor window",
            };

            lock (_windowsLock)
            {
                // The app may have been disposed meanwhile
                if (!_running)
                    throw OpenWindowNeedsRun();
                thread.Start();
                started = true;
                _openWindows.Add((window, closed.Task));
            }
            return window;
        }
        catch
        {
            if (!started)
            {
                if (manager is not null)
                    DisposeWindowAsync(window, manager).GetAwaiter().GetResult();
                else
                    window.Dispose();
            }
            throw;
        }
    }

    private static InvalidOperationException OpenWindowNeedsRun() =>
        new("OpenWindow() needs the main window: call it once Run() started, e.g. from a component.");

    // Disposes the components of the window, then the window
    private async Task DisposeWindowAsync(RustinoWindow window, RustinoWebViewManager manager)
    {
        try { await manager.DisposeAsync().ConfigureAwait(false); }
        catch (Exception exception) { ReportWindowFailed(window, exception, "Disposing a window opened with OpenWindow failed."); }
        finally
        {
            try { window.Dispose(); }
            catch (Exception exception) { Log(exception, "Disposing a window opened with OpenWindow failed."); }
        }
    }

    private void ReportWindowFailed(RustinoWindow window, Exception exception, string message)
    {
        Log(exception, message);
        try { WindowFailed?.Invoke(this, new WindowFailedEventArgs(window, exception)); }
        catch (Exception handlerException) { Log(handlerException, "Unhandled exception in a WindowFailed handler."); }
    }

    private void Log(Exception exception, string message)
    {
        try { _logger?.LogError(exception, message); }
        catch (Exception) { }
    }
}

public sealed class WindowFailedEventArgs(RustinoWindow window, Exception exception) : EventArgs
{
    public RustinoWindow Window { get; } = window;

    public Exception Exception { get; } = exception;
}
