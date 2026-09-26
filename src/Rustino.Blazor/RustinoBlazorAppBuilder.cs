using System.Collections;
using Microsoft.AspNetCore.Components;
using Microsoft.AspNetCore.Components.Web;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.FileProviders;
using Rustino.NET;

namespace Rustino.Blazor;

public class RustinoBlazorAppBuilder
{
    private RustinoBlazorAppBuilder(IFileProvider? fileProvider)
    {
        Services
            .AddBlazorWebView()
            .AddSingleton<RustinoWindow>()
            // RustinoWindow calls are queued to the event loop from any thread,
            // so Blazor doesn't need a UI-thread dispatcher
            .AddSingleton(Dispatcher.CreateDefault())
            .AddSingleton(fileProvider ?? DefaultFileProvider())
            .AddSingleton<JSComponentConfigurationStore>()
            .AddSingleton<RustinoWebViewManager>()
            .AddScoped(sp => new HttpClient(new RustinoHttpHandler(sp.GetRequiredService<RustinoWebViewManager>()))
            {
                BaseAddress = RustinoWebViewManager.AppBaseUri
            });
    }

    public static RustinoBlazorAppBuilder CreateDefault(string[]? args = null) => CreateDefault(null, args);

    // `args` is accepted for parity with the other Blazor hosts and is currently unused.
    // Static files are served from `fileProvider`, or by default from the app's `wwwroot`.
    public static RustinoBlazorAppBuilder CreateDefault(IFileProvider? fileProvider, string[]? args = null) =>
        new(fileProvider);

    public IServiceCollection Services { get; } = new ServiceCollection();

    public RootComponentList RootComponents { get; } = new();

    public RustinoBlazorApp Build() => new(Services.BuildServiceProvider(), RootComponents);

    // Published apps ship `wwwroot` next to the executable. During development the WebView
    // static web assets manifest serves the project's `wwwroot` and `_framework` files instead.
    private static IFileProvider DefaultFileProvider()
    {
        var root = Path.Combine(AppContext.BaseDirectory, "wwwroot");
        return Directory.Exists(root) ? new PhysicalFileProvider(root) : new NullFileProvider();
    }
}

public class RootComponentList : IEnumerable<(Type ComponentType, string Selector)>
{
    private readonly List<(Type, string)> _components = new();

    public void Add<TComponent>(string selector) where TComponent : IComponent =>
        _components.Add((typeof(TComponent), selector));

    public IEnumerator<(Type ComponentType, string Selector)> GetEnumerator() => _components.GetEnumerator();

    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();
}
