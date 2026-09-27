namespace Rustino.NET.Tests.Interop;

internal static class RepoPaths
{
    private static readonly Lazy<string> RootLazy = new(FindRoot);

    /// <summary>The repository root: the first ancestor of the test binaries containing src/Rustino.Native/Cargo.toml.</summary>
    public static string Root => RootLazy.Value;

    public static string NativeCrate => Path.Combine(Root, "src", "Rustino.Native");

    private static string FindRoot()
    {
        foreach (var start in new[] { AppContext.BaseDirectory, Directory.GetCurrentDirectory() })
        {
            for (var dir = new DirectoryInfo(start); dir is not null; dir = dir.Parent)
            {
                if (File.Exists(Path.Combine(dir.FullName, "src", "Rustino.Native", "Cargo.toml")))
                    return dir.FullName;
            }
        }
        throw new DirectoryNotFoundException(
            $"Cannot find the repository root (a directory containing src/Rustino.Native/Cargo.toml) above {AppContext.BaseDirectory}");
    }
}
