namespace Rustino.NET.Tests;

/// <summary>
/// The parts of RustinoAutoStart that don't touch the system: the entries themselves are checked by hand, since
/// writing them would show the test app in the user's startup apps (and in a macOS notification).
/// </summary>
public class AutoStartTests
{
    [Theory]
    [InlineData("com.example.App")]
    [InlineData("my-app_2")]
    public void ReverseDnsIdsAreValid(string appId) => Assert.True(RustinoAutoStart.IsValidAppId(appId));

    [Theory]
    [InlineData("")]
    [InlineData(".hidden")]
    [InlineData("a b")]
    [InlineData("a/b")]
    [InlineData(@"a\b")]
    [InlineData("città")]
    public void IdsThatCantNameAFileAreRejected(string appId)
    {
        Assert.False(RustinoAutoStart.IsValidAppId(appId));
        Assert.Throws<ArgumentException>(() => new RustinoAutoStart(appId));
    }

    [Fact]
    public void TheCommandEndsWithTheLaunchArgument()
    {
        var executable = Path.Combine(Path.GetTempPath(), "My App", "app");
        using var autoStart = new RustinoAutoStart("com.example.App",
            new AutoStartOptions { ExecutablePath = executable, Arguments = ["--profile", "work"] });

        Assert.Equal(string.Join('\n', executable, "--profile", "work", RustinoAutoStart.LaunchArgument), autoStart.Command);
    }

    [Fact]
    public void TheRunningExecutableIsTheDefault()
    {
        using var autoStart = new RustinoAutoStart("com.example.App");

        Assert.StartsWith(Environment.ProcessPath + "\n", autoStart.Command);
        Assert.EndsWith("\n" + RustinoAutoStart.LaunchArgument, autoStart.Command);
    }

    [Fact]
    public void ArgumentsWithLineBreaksAreRejected() =>
        Assert.Throws<ArgumentException>(() => new RustinoAutoStart("com.example.App",
            new AutoStartOptions { Arguments = ["a\nb"] }));

    [Fact]
    public void TheTestHostWasNotLaunchedAtLogin() => Assert.False(RustinoAutoStart.WasLaunchedAtLogin);
}
