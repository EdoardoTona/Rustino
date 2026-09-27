namespace Rustino.NET.Tests;

public class SingleInstanceTests
{
    [Fact]
    public void HandlerCanDisposeItsOwnPrimary()
    {
        var appId = $"rustino.test.{Guid.NewGuid():N}";
        using var releaseHandler = new ManualResetEventSlim();
        using var disposed = new ManualResetEventSlim();
        var primary = RustinoSingleInstance.Acquire(appId);
        primary.SecondInstanceStarted += (_, _) =>
        {
            if (!releaseHandler.Wait(TimeSpan.FromSeconds(10)))
                throw new TimeoutException("The handler was not released.");
            primary.Dispose();
            disposed.Set();
        };

        try
        {
            using var secondary = RustinoSingleInstance.Acquire(appId, new SingleInstanceOptions
            {
                Arguments = ["launch"],
                Timeout = TimeSpan.FromSeconds(5),
            });
            Assert.Equal(SingleInstanceStatus.Forwarded, secondary.Status);
            releaseHandler.Set();
            Assert.True(disposed.Wait(TimeSpan.FromSeconds(5)), "Dispose deadlocked in its own handler.");
        }
        finally
        {
            releaseHandler.Set();
            primary.Dispose();
        }
    }

    [Fact]
    public async Task DisposeDispatchesLaunchesThatWereAlreadyAcknowledged()
    {
        var appId = $"rustino.test.{Guid.NewGuid():N}";
        using var firstEntered = new ManualResetEventSlim();
        using var releaseFirst = new ManualResetEventSlim();
        using var secondHandled = new ManualResetEventSlim();
        using var disposeStarted = new ManualResetEventSlim();
        var primary = RustinoSingleInstance.Acquire(appId);
        var handled = 0;
        primary.SecondInstanceStarted += (_, _) =>
        {
            if (Interlocked.Increment(ref handled) == 1)
            {
                firstEntered.Set();
                if (!releaseFirst.Wait(TimeSpan.FromSeconds(10)))
                    throw new TimeoutException("The first launch was not released.");
            }
            else
            {
                secondHandled.Set();
            }
        };

        try
        {
            using var first = RustinoSingleInstance.Acquire(appId, new SingleInstanceOptions
            {
                Arguments = ["first"],
                Timeout = TimeSpan.FromSeconds(5),
            });
            Assert.Equal(SingleInstanceStatus.Forwarded, first.Status);
            Assert.True(firstEntered.Wait(TimeSpan.FromSeconds(5)));

            using var second = RustinoSingleInstance.Acquire(appId, new SingleInstanceOptions
            {
                Arguments = ["second"],
                Timeout = TimeSpan.FromSeconds(5),
            });
            Assert.Equal(SingleInstanceStatus.Forwarded, second.Status);

            var disposing = Task.Run(() =>
            {
                disposeStarted.Set();
                primary.Dispose();
            });
            Assert.True(disposeStarted.Wait(TimeSpan.FromSeconds(5)));
            await Task.Delay(100);
            Assert.False(disposing.IsCompleted,
                "Dispose must wait for the acknowledged launch to be dispatched.");
            releaseFirst.Set();
            await disposing.WaitAsync(TimeSpan.FromSeconds(10));
            Assert.True(secondHandled.IsSet, "Dispose returned before dispatching the acknowledged launch.");
            Assert.Equal(2, Volatile.Read(ref handled));
        }
        finally
        {
            releaseFirst.Set();
            primary.Dispose();
        }
    }
}
