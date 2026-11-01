namespace Rustino.NET.Tests;

/// <summary>
/// Where WebView2 keeps its data when the app doesn't call SetUserDataFolder (see
/// RustinoWindow.DefaultUserDataFolder). Pure path logic, so it runs on every platform.
/// </summary>
public class UserDataFolderTests
{
    private static readonly string LocalAppData = Path.Combine(Path.GetTempPath(), "LocalAppData");
    private static readonly string AppDir = Path.Combine(Path.GetTempPath(), "MyApp");
    private static readonly string DotnetDir = Path.Combine(Path.GetTempPath(), "Program Files", "dotnet");

    [Fact]
    public void AnAppThatCanWriteNextToItsExecutableKeepsTheWebView2Default()
    {
        var probed = new List<string>();
        var folder = RustinoWindow.DefaultUserDataFolder(Path.Combine(AppDir, "MyApp.exe"), "MyApp", LocalAppData,
            path => { probed.Add(path); return true; });

        Assert.Null(folder);
        Assert.Equal([Path.Combine(AppDir, "MyApp.WebView2")], probed);
    }

    [Fact]
    public void AnAppThatCannotWriteNextToItsExecutableUsesLocalAppData()
    {
        var folder = RustinoWindow.DefaultUserDataFolder(Path.Combine(AppDir, "MyApp.exe"), "MyApp", LocalAppData, _ => false);

        Assert.Equal(Path.Combine(LocalAppData, "MyApp", "WebView2"), folder);
    }

    [Theory]
    [InlineData("dotnet.exe")]
    [InlineData("DOTNET.EXE")]
    public void DotnetAppDllUsesLocalAppDataUnderTheAppName(string host)
    {
        var folder = RustinoWindow.DefaultUserDataFolder(Path.Combine(DotnetDir, host), "MyApp", LocalAppData,
            _ => throw new InvalidOperationException("dotnet's folder is shared by every app: never use it"));

        Assert.Equal(Path.Combine(LocalAppData, "MyApp", "WebView2"), folder);
    }

    [Fact]
    public void WithoutAnEntryAssemblyTheExecutableNameIsUsed()
    {
        var folder = RustinoWindow.DefaultUserDataFolder(Path.Combine(DotnetDir, "dotnet.exe"), null, LocalAppData, _ => false);

        Assert.Equal(Path.Combine(LocalAppData, "dotnet", "WebView2"), folder);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    public void WithoutAProcessPathNothingChanges(string? processPath)
    {
        Assert.Null(RustinoWindow.DefaultUserDataFolder(processPath, "MyApp", LocalAppData, _ => false));
    }
}
