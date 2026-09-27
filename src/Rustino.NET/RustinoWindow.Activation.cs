namespace Rustino.NET;

public partial class RustinoWindow
{
    /// <summary>
    /// Shows the window if hidden, restores it if minimized, and asks the operating system to focus it.
    /// On Linux, pass the launcher's activation token when available to help the desktop grant focus.
    /// Calls made before the window starts are queued.
    /// </summary>
    /// <param name="activationToken">Optional Linux desktop activation token.</param>
    public RustinoWindow Activate(string? activationToken = null)
    {
        ThrowIfDisposed();
        EnsureNative();
        RustinoDllImports.rustino_activate(_nativeHandle, activationToken);
        return this;
    }
}
