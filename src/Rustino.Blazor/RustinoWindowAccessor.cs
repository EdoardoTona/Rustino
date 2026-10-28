using Rustino.NET;

namespace Rustino.Blazor;

/// <summary>
/// The window of the page, for scoped services: inject it instead of <see cref="RustinoWindow"/>,
/// which the container resolves to the main window in their constructors. Components can inject
/// either one.
/// </summary>
public sealed class RustinoWindowAccessor
{
    internal RustinoWindowAccessor(RustinoWindow window) => Window = window;

    // Not disposable: disposing a scope leaves the window alone
    public RustinoWindow Window { get; internal set; }
}
