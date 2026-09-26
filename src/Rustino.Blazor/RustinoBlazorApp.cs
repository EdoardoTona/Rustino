using Microsoft.AspNetCore.Components;
using Microsoft.Extensions.DependencyInjection;
using Rustino.NET;

namespace Rustino.Blazor;

public class RustinoBlazorApp
{
    private readonly RustinoWebViewManager _manager;

    internal RustinoBlazorApp(IServiceProvider services, RootComponentList rootComponents)
    {
        Services = services;
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

    public void Run()
    {
        _manager.Navigate("/");
        MainWindow.WaitForClose();
    }
}
