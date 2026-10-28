using Microsoft.AspNetCore.Components;
using Microsoft.AspNetCore.Components.Web;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.FileProviders;
using Microsoft.Extensions.Logging;
using Rustino.NET;

namespace Rustino.Blazor;

public class RustinoBlazorApp
{
    private readonly RustinoWebViewManager _manager;
    private readonly ILogger<RustinoBlazorApp>? _logger;
    private volatile bool _running;

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

    public void Run()
    {
        _running = true;
        try
        {
            _manager.Navigate("/");
            MainWindow.WaitForClose();
        }
        finally
        {
            _running = false;
        }
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
        if (!_running)
            throw new InvalidOperationException("OpenWindow() needs the main window: call it once Run() started, e.g. from a component.");

        var window = new RustinoWindow()
            .SetTitle("Rustino.Blazor App")
            .SetUseOsDefaultSize(false)
            .SetSize(800, 600)
            .Center();
        configure?.Invoke(window);

        var manager = new RustinoWebViewManager(
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
                // Nothing may escape this thread: it would end the process
                Log(exception, "A window opened with OpenWindow failed.");
                try { WindowFailed?.Invoke(this, new WindowFailedEventArgs(window, exception)); }
                catch (Exception handlerException) { Log(handlerException, "Unhandled exception in a WindowFailed handler."); }
            }
            finally
            {
                // Disposes the components of the window, then the window
                _ = manager.DisposeAsync().AsTask().ContinueWith(_ => window.Dispose(), TaskScheduler.Default);
            }
        })
        {
            IsBackground = true,
            Name = "Rustino.Blazor window",
        };
        thread.Start();
        return window;
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
