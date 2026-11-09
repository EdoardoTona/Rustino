using Rustino.NET.Tests.Interop;

namespace Rustino.NET.Tests;

/// <summary>
/// Windows started while the first one runs join its event loop: their events come from its
/// thread, and they close when it closes. Opens hidden windows, so it needs a desktop session
/// and the WebView2 Runtime.
/// </summary>
public class MultiWindowTests
{
    private static readonly TimeSpan Timeout = TimeSpan.FromSeconds(30);

    // The page tells the host it loaded, with the thread raising the event
    private sealed class TestWindow : IDisposable
    {
        public readonly RustinoWindow Window = new RustinoWindow()
            .SetVisible(false)
            .Load("data:text/html," + Uri.EscapeDataString("<script>window.ipc.postMessage('ready')</script>"));
        public readonly TaskCompletionSource<int> Ready = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public readonly TaskCompletionSource Closed = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task Run = Task.CompletedTask;

        public TestWindow()
        {
            Window.WebMessageReceived += (_, _) => Ready.TrySetResult(Environment.CurrentManagedThreadId);
            Window.WindowClosed += (_, _) => Closed.TrySetResult();
        }

        public void Start() => Run = Task.Factory.StartNew(Window.WaitForClose, TaskCreationOptions.LongRunning);

        public void Dispose() => Window.Dispose();
    }

    [WindowsNativeLibraryFact]
    public async Task WindowsJoinTheEventLoopOfTheFirstAndCloseWithIt()
    {
        using TestWindow main = new(), second = new(), third = new();
        main.Start();
        var loopThread = await main.Ready.Task.WaitAsync(Timeout);

        second.Start();
        Assert.Equal(loopThread, await second.Ready.Task.WaitAsync(Timeout));
        second.Window.Close();
        await second.Run.WaitAsync(Timeout);
        await second.Closed.Task.WaitAsync(Timeout);
        Assert.False(main.Run.IsCompleted, "the first window stays open");

        third.Start();
        await third.Ready.Task.WaitAsync(Timeout);
        main.Window.Close();
        await Task.WhenAll(main.Run, third.Run).WaitAsync(Timeout);
        Assert.True(third.Closed.Task.IsCompleted, "closed with the first window");
    }
}
