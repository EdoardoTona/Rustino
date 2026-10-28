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

    [Fact]
    public void OpenWindowNeedsTheAppToRun()
    {
        var app = RustinoBlazorAppBuilder.CreateDefault().Build();

        var error = Assert.Throws<InvalidOperationException>(() => app.OpenWindow<Empty>("#app"));
        Assert.Contains("Run()", error.Message);
    }
}
