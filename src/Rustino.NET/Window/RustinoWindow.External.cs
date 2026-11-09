namespace Rustino.NET;

public partial class RustinoWindow
{
    /// <summary>Opens an http, https or mailto URL in the system's default application.</summary>
    /// <returns>True when the system accepted the request to open the URL.</returns>
    public static bool OpenExternal(string url)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(url);
        NativeLibraryResolver.EnsureRegistered();
        return RustinoDllImports.rustino_open_external(url) != 0;
    }
}
