using Microsoft.AspNetCore.Components;
using Microsoft.Extensions.DependencyInjection;

namespace Rustino.Blazor.Tests;

public class BlazorAppTests
{
    private sealed class Empty : ComponentBase;

    [Fact]
    public void ComponentsCanInjectTheApp()
    {
        var app = RustinoBlazorAppBuilder.CreateDefault().Build();

        Assert.Same(app, app.Services.GetRequiredService<RustinoBlazorApp>());
    }

    private sealed class DisposableService : IDisposable
    {
        public bool Disposed { get; private set; }

        public void Dispose() => Disposed = true;
    }

    [Fact]
    public async Task DisposingTheAppDisposesItsServicesOnce()
    {
        var builder = RustinoBlazorAppBuilder.CreateDefault();
        builder.Services.AddSingleton<DisposableService>();
        var app = builder.Build();
        var service = app.Services.GetRequiredService<DisposableService>();

        // The service provider disposes the app too: that must not wait for the app's own disposal
        await app.DisposeAsync().AsTask().WaitAsync(TimeSpan.FromSeconds(10));

        Assert.True(service.Disposed);
        Assert.Throws<ObjectDisposedException>(() => app.MainWindow.SetTitle("disposed"));
        app.Dispose();
        Assert.Throws<ObjectDisposedException>(app.Run);
    }

    [Fact]
    public async Task DisposeOnTheRenderersDispatcherDoesNotDeadlock()
    {
        var builder = RustinoBlazorAppBuilder.CreateDefault();
        builder.Services.AddSingleton<DisposableService>();
        var app = builder.Build();
        var service = app.Services.GetRequiredService<DisposableService>();
        var dispatcher = app.Services.GetRequiredService<Dispatcher>();

        // E.g. a component disposing the app: the disposal needs the dispatcher, so it can't be waited for there
        await dispatcher.InvokeAsync(app.Dispose).WaitAsync(TimeSpan.FromSeconds(10));
        await app.DisposeAsync().AsTask().WaitAsync(TimeSpan.FromSeconds(10));

        Assert.True(service.Disposed);
    }

    [Fact]
    public void OpenWindowNeedsTheAppToRun()
    {
        var app = RustinoBlazorAppBuilder.CreateDefault().Build();

        var error = Assert.Throws<InvalidOperationException>(() => app.OpenWindow<Empty>("#app"));
        Assert.Contains("Run()", error.Message);
    }
}
