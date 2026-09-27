namespace Rustino.NET.Tests.Interop;

/// <summary>The native library built for the current platform (cargo build --release).</summary>
internal static class NativeBinary
{
    /// <summary>Overrides the library to check, e.g. a build for another target directory.</summary>
    public const string PathVariable = "RUSTINO_NATIVE_LIB";

    public static string FileName =>
        OperatingSystem.IsWindows() ? "rustino_native.dll"
        : OperatingSystem.IsMacOS() ? "librustino_native.dylib"
        : "librustino_native.so";

    public static string ExpectedPath =>
        Environment.GetEnvironmentVariable(PathVariable) is { Length: > 0 } custom
            ? Path.GetFullPath(custom)
            : Path.Combine(RepoPaths.NativeCrate, "target", "release", FileName);

    /// <summary>A hint when the library is older than the Rust sources, which explains most failures.</summary>
    public static string StalenessHint(string libraryPath)
    {
        var built = File.GetLastWriteTimeUtc(libraryPath);
        var newest = Directory.EnumerateFiles(Path.Combine(RepoPaths.NativeCrate, "src"), "*.rs", SearchOption.AllDirectories)
            .Append(Path.Combine(RepoPaths.NativeCrate, "Cargo.toml"))
            .Select(f => (File: f, Time: File.GetLastWriteTimeUtc(f)))
            .MaxBy(x => x.Time);
        return newest.Time > built
            ? $"\nNote: {libraryPath} was built before {Path.GetRelativePath(RepoPaths.NativeCrate, newest.File)} was last changed: run `cargo build --release` in src/Rustino.Native."
            : "";
    }
}

/// <summary>A [Fact] that is skipped, with the reason, when the native library has not been built.</summary>
internal sealed class NativeLibraryFactAttribute : FactAttribute
{
    public NativeLibraryFactAttribute()
    {
        string path;
        try
        {
            path = NativeBinary.ExpectedPath;
        }
        catch (DirectoryNotFoundException e)
        {
            Skip = e.Message;
            return;
        }
        if (!File.Exists(path))
            Skip = $"Native library not built: {path} does not exist. Run `cargo build --release` in src/Rustino.Native " +
                   $"(or set {NativeBinary.PathVariable} to the library to check).";
    }
}
