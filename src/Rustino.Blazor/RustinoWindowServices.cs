using Microsoft.Extensions.DependencyInjection;
using Rustino.NET;

namespace Rustino.Blazor;

// The services of a window opened with RustinoBlazorApp.OpenWindow: the app's services, except
// that RustinoWindow resolves to this window. Its scopes (one per page) do the same, so the
// components of the window that inject RustinoWindow get it, and set RustinoWindowAccessor for the
// scoped services, which the container builds with the main window.
internal sealed class RustinoWindowServices(IServiceProvider services, RustinoWindow window)
    : IServiceProvider, IServiceScopeFactory
{
    public object? GetService(Type serviceType) =>
        serviceType == typeof(RustinoWindow) ? window
        : serviceType == typeof(IServiceScopeFactory) ? this
        : services.GetService(serviceType);

    public IServiceScope CreateScope()
    {
        var scope = services.CreateAsyncScope();
        // Before any service of the scope gets it
        scope.ServiceProvider.GetRequiredService<RustinoWindowAccessor>().Window = window;
        return new Scope(scope, window);
    }

    private sealed class Scope(AsyncServiceScope scope, RustinoWindow window) : IServiceScope, IAsyncDisposable
    {
        public IServiceProvider ServiceProvider { get; } = new RustinoWindowServices(scope.ServiceProvider, window);

        public void Dispose() => scope.Dispose();

        public ValueTask DisposeAsync() => scope.DisposeAsync();
    }
}
