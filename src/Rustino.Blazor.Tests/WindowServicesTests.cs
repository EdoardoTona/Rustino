using Microsoft.AspNetCore.Components;
using Microsoft.Extensions.DependencyInjection;
using Rustino.NET;

namespace Rustino.Blazor.Tests;

public class WindowServicesTests
{
    private sealed class Shared;

    private sealed class PerPage;

    private sealed class Empty : ComponentBase;

    // A scoped service that needs the window of its page
    private sealed class Editor(RustinoWindowAccessor accessor)
    {
        public RustinoWindow Window => accessor.Window;
    }

    private sealed class Disposable : IDisposable
    {
        public bool Disposed { get; private set; }

        public void Dispose() => Disposed = true;
    }

    // The app's registrations, as RustinoBlazorAppBuilder makes them
    private static RustinoBlazorApp App()
    {
        var builder = RustinoBlazorAppBuilder.CreateDefault();
        builder.Services
            .AddSingleton<Shared>()
            .AddScoped<PerPage>()
            .AddScoped<Editor>()
            .AddScoped<Disposable>();
        return builder.Build();
    }

    // As WebViewManager keeps them
    private static IServiceProvider WindowServices(RustinoBlazorApp app, RustinoWindow window) =>
        new RustinoWindowServices(app.Services, window);

    [Fact]
    public void RustinoWindowIsTheWindowInItsScopesToo()
    {
        var app = App();
        using var window = new RustinoWindow();
        var services = WindowServices(app, window);

        Assert.Same(window, services.GetService<RustinoWindow>());
        Assert.Same(app.MainWindow, app.Services.GetService<RustinoWindow>());

        // WebViewManager creates one scope per page
        using var scope = services.CreateScope();
        Assert.Same(window, scope.ServiceProvider.GetService<RustinoWindow>());
        Assert.Same(window, scope.ServiceProvider.CreateScope().ServiceProvider.GetService<RustinoWindow>());
    }

    [Fact]
    public void ScopedServicesGetTheirWindowThroughTheAccessor()
    {
        var app = App();
        using var window = new RustinoWindow();

        using (var main = app.Services.CreateScope())
            Assert.Same(app.MainWindow, main.ServiceProvider.GetRequiredService<Editor>().Window);

        using var scope = WindowServices(app, window).CreateScope();
        Assert.Same(window, scope.ServiceProvider.GetRequiredService<Editor>().Window);
        Assert.Same(window, scope.ServiceProvider.GetRequiredService<RustinoWindowAccessor>().Window);
    }

    [Fact]
    public void ReloadingAPageKeepsItsWindow()
    {
        var app = App();
        using var window = new RustinoWindow();
        var services = WindowServices(app, window);

        // A reload disposes the scope of the page and creates another one
        var first = services.CreateScope();
        var firstEditor = first.ServiceProvider.GetRequiredService<Editor>();
        first.Dispose();
        using var second = services.CreateScope();
        var secondEditor = second.ServiceProvider.GetRequiredService<Editor>();

        Assert.NotSame(firstEditor, secondEditor);
        Assert.Same(window, secondEditor.Window);
        // Neither window is disposed with the scopes
        window.SetTitle("still alive");
        using (var main = app.Services.CreateScope())
            _ = main.ServiceProvider.GetRequiredService<Editor>();
        app.MainWindow.SetTitle("still alive");
    }

    [Fact]
    public void OtherServicesComeFromTheApp()
    {
        var app = App();
        using var window = new RustinoWindow();
        using var scope = WindowServices(app, window).CreateScope();
        using var appScope = app.Services.CreateScope();

        Assert.Same(app.Services.GetService<Shared>(), scope.ServiceProvider.GetService<Shared>());
        Assert.NotSame(appScope.ServiceProvider.GetService<PerPage>(), scope.ServiceProvider.GetService<PerPage>());
        Assert.Same(scope.ServiceProvider.GetService<PerPage>(), scope.ServiceProvider.GetService<PerPage>());
        Assert.Null(scope.ServiceProvider.GetService<Empty>());
    }

    [Fact]
    public async Task ScopesDisposeTheirServices()
    {
        var app = App();
        using var window = new RustinoWindow();
        var scope = WindowServices(app, window).CreateAsyncScope();
        var service = scope.ServiceProvider.GetRequiredService<Disposable>();

        await scope.DisposeAsync();
        Assert.True(service.Disposed);
    }
}
