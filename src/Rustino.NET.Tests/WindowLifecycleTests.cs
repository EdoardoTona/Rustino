using System.Reflection;

namespace Rustino.NET.Tests;

/// <summary>Window behavior that doesn't need the native window: disposal, streams, temporary files, URLs.</summary>
public class WindowLifecycleTests
{
    private sealed class Observer<T>(bool throwOnCompleted = false) : IObserver<T>
    {
        public bool Completed { get; private set; }

        public void OnCompleted()
        {
            Completed = true;
            if (throwOnCompleted) throw new InvalidOperationException("observer failed");
        }

        public void OnError(Exception error) { }

        public void OnNext(T value) { }
    }

    [Fact]
    public void DisposeCompletesEveryStreamWhenAnObserverThrows()
    {
        var window = new RustinoWindow();
        var reports = new List<NativeCallbackExceptionEventArgs>();
        window.UnhandledCallbackException += (_, e) => reports.Add(e);
        var failing = new Observer<(int, int)>(throwOnCompleted: true);
        var sameStream = new Observer<(int, int)>();
        var otherStream = new Observer<(int, int)>();
        window.WhenSizeChanged.Subscribe(failing);
        window.WhenSizeChanged.Subscribe(sameStream);
        window.WhenLocationChanged.Subscribe(otherStream);

        window.Dispose();

        Assert.True(sameStream.Completed);
        Assert.True(otherStream.Completed);
        Assert.Equal("observer failed", Assert.Single(reports).Exception.Message);
    }

    [Fact]
    public void IconsWrittenFromStreamsAreDeletedWhenReplacedAndOnDispose()
    {
        var window = new RustinoWindow();
        var field = typeof(RustinoWindow).GetField("_iconTempFile", BindingFlags.Instance | BindingFlags.NonPublic)!;
        string TempIcon() => (string)field.GetValue(window)!;

        window.SetIcon(new MemoryStream([1, 2, 3]));
        var first = TempIcon();
        window.SetIcon(new MemoryStream([4, 5, 6]));
        var second = TempIcon();
        Assert.False(File.Exists(first));
        Assert.True(File.Exists(second));

        window.SetIconFile("icon.png");
        Assert.False(File.Exists(second));

        window.SetIcon(new MemoryStream([7, 8, 9]));
        var third = TempIcon();
        window.Dispose();
        Assert.False(File.Exists(third));
    }

    [Theory]
    [InlineData("https://example.com/page?query=1#top")]
    [InlineData("app://localhost/")]
    [InlineData("about:blank")]
    public void AbsoluteUrlsAreLoadedAsTheyAre(string url) => Assert.Equal(url, new RustinoWindow().ToUrl(url));

    [Theory]
    [InlineData("index.html")]
    [InlineData("wwwroot/second.html")]
    [InlineData("dir with spaces/pàgina #1.html")]
    public void RelativePathsAreServedFromTheCurrentDirectory(string path)
    {
        var window = new RustinoWindow();

        var url = window.ToUrl(path);

        Assert.StartsWith("rustino-file://localhost/", url);
        Assert.DoesNotContain(' ', url);
        Assert.Equal(Path.GetFullPath(path), RustinoWindow.LocalFilePathOf(url, [Path.GetDirectoryName(Path.GetFullPath(path))!]));
    }

    [Fact]
    public void AbsolutePathsAndFileUrlsAreServed()
    {
        var path = Path.Combine(Path.GetTempPath(), "rustino page.html");
        var window = new RustinoWindow();

        Assert.Equal(RustinoWindow.LocalFileUrlOf(path), window.ToUrl(path));
        Assert.Equal(RustinoWindow.LocalFileUrlOf(path) + "?a=1#top", window.ToUrl(new Uri(path).AbsoluteUri + "?a=1#top"));
    }

    [Fact]
    public void OnlyTheFoldersOfLoadedFilesAreServed()
    {
        var root = Path.Combine(Path.GetTempPath(), "rustino-app");
        string[] roots = [root];
        string Url(params string[] parts) => RustinoWindow.LocalFileUrlOf(Path.Combine([root, .. parts]));

        Assert.Equal(Path.Combine(root, "css", "app.css"), RustinoWindow.LocalFilePathOf(Url("css", "app.css"), roots));
        Assert.Null(RustinoWindow.LocalFilePathOf(RustinoWindow.LocalFileUrlOf(Path.Combine(Path.GetTempPath(), "other.txt")), roots));
        Assert.Null(RustinoWindow.LocalFilePathOf(Url("..", "rustino-app-secret", "key.txt"), roots));
        Assert.Null(RustinoWindow.LocalFilePathOf(Url("..", "..", "etc", "passwd").Replace("/..", "/%2E%2E"), roots));
        Assert.Null(RustinoWindow.LocalFilePathOf("https://example.com" + new Uri(Url("index.html")).AbsolutePath, roots));
    }

    [Fact]
    public void LinksOutOfTheServedFoldersAreNotFollowed()
    {
        var temp = Directory.CreateTempSubdirectory("rustino-links-").FullName;
        try
        {
            var served = Directory.CreateDirectory(Path.Combine(temp, "served")).FullName;
            var outside = Directory.CreateDirectory(Path.Combine(temp, "outside")).FullName;
            File.WriteAllText(Path.Combine(outside, "private.txt"), "secret");
            File.WriteAllText(Path.Combine(served, "page.html"), "page");
            Directory.CreateDirectory(Path.Combine(served, "inside"));
            File.WriteAllText(Path.Combine(served, "inside", "file.txt"), "ok");
            try
            {
                Directory.CreateSymbolicLink(Path.Combine(served, "link"), outside);
                File.CreateSymbolicLink(Path.Combine(served, "file-link.txt"), Path.Combine(outside, "private.txt"));
                Directory.CreateSymbolicLink(Path.Combine(served, "inner-link"), Path.Combine(served, "inside"));
            }
            catch (UnauthorizedAccessException) { return; } // Windows without the privilege to create links
            catch (IOException) when (OperatingSystem.IsWindows()) { return; }
            string[] roots = [served];
            string? PathOf(params string[] parts) =>
                RustinoWindow.LocalFilePathOf(RustinoWindow.LocalFileUrlOf(Path.Combine([served, .. parts])), roots);

            Assert.NotNull(PathOf("page.html"));
            Assert.Null(PathOf("link", "private.txt"));
            Assert.Null(PathOf("file-link.txt"));
            // Links that stay within the served folders are fine
            Assert.NotNull(PathOf("inner-link", "file.txt"));
        }
        finally
        {
            Directory.Delete(temp, recursive: true);
        }
    }

    [Theory]
    [InlineData("literal%20name.html")]
    [InlineData("100% done.html")]
    [InlineData("a%2Fb.html")]
    [InlineData("question?.html")]
    public void FileNamesAreKeptInUrls(string name)
    {
        if (OperatingSystem.IsWindows() && name.Contains('?')) return; // not a valid Windows file name
        var root = Path.Combine(Path.GetTempPath(), "rustino-app");
        var path = Path.Combine(root, name);

        Assert.Equal(path, RustinoWindow.LocalFilePathOf(RustinoWindow.LocalFileUrlOf(path), [root]));
    }

    [Fact]
    public void TheLocalFileSchemeIsReserved() =>
        Assert.Throws<ArgumentException>(() => new RustinoWindow().RegisterCustomSchemeHandler(
            "rustino-file", (object _, string _, string _, out string? contentType) => { contentType = null; return null; }));

    [Fact]
    public void SplashscreenIsOnlySupportedOnWindows()
    {
        Assert.Equal(OperatingSystem.IsWindows(), RustinoSplashscreen.IsSupported);
        if (!RustinoSplashscreen.IsSupported)
            Assert.Throws<PlatformNotSupportedException>(() => new RustinoSplashscreen("splash.png"));
    }
}
