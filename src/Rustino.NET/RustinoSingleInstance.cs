using System.Diagnostics;
using System.IO.Pipes;
using System.Threading.Channels;
using Microsoft.Extensions.Logging;

namespace Rustino.NET;

/// <summary>
/// Keeps one process active for an app id. Acquire this before creating windows and keep the returned
/// object alive for as long as the process should receive later launches.
/// </summary>
public sealed class RustinoSingleInstance : IDisposable
{
    private readonly object _gate = new();
    private readonly SingleInstanceEndpoint _endpoint;
    private readonly SingleInstanceOptions _options;
    private readonly string[] _arguments;
    private readonly FileStream? _lockFile;
    private readonly Channel<SecondInstanceEventArgs> _messages = Channel.CreateBounded<SecondInstanceEventArgs>(
        new BoundedChannelOptions(256) { FullMode = BoundedChannelFullMode.Wait, SingleReader = true, SingleWriter = false });
    private EventHandler<SecondInstanceEventArgs>? _secondInstanceStarted;
    private EventHandler<SingleInstanceExceptionEventArgs>? _unhandledException;
    private RustinoWindow? _mainWindow;
    private SingleInstanceServer? _server;
    private Task? _dispatcher;
    private int _dispatchThreadId;
    private int _disposed;

    private RustinoSingleInstance(
        string appId,
        SingleInstanceStatus status,
        SingleInstanceEndpoint endpoint,
        SingleInstanceOptions options,
        string[] arguments,
        FileStream? lockFile)
    {
        AppId = appId;
        Status = status;
        _endpoint = endpoint;
        _options = options;
        _arguments = arguments;
        _lockFile = lockFile;
    }

    /// <summary>Claims the app id or forwards this launch to the process that already owns it.</summary>
    public static RustinoSingleInstance Acquire(string appId, SingleInstanceOptions? options = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(appId);
        if (appId.Length > 256 || appId.Contains('\0'))
            throw new ArgumentException("The app id must contain 1 to 256 characters and no NUL characters.", nameof(appId));

        options ??= new SingleInstanceOptions();
        ValidateTimeout(options.Timeout, nameof(options.Timeout));
        var endpoint = SingleInstanceEndpoint.Create(appId);
        var arguments = (options.Arguments ?? Environment.GetCommandLineArgs().Skip(1).ToArray()).ToArray();
        if (arguments.Any(argument => argument is null))
            throw new ArgumentException("Arguments cannot contain null values.", nameof(options));
        var deadline = DateTime.UtcNow + options.Timeout;

        while (true)
        {
            FileStream? lockFile;
            try
            {
                lockFile = new FileStream(endpoint.LockPath, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);
            }
            catch (Exception exception) when (exception is IOException or UnauthorizedAccessException)
            {
                lockFile = null;
            }

            if (lockFile is not null && !endpoint.TryLockUnix(lockFile))
            {
                lockFile.Dispose();
                lockFile = null;
            }

            if (lockFile is not null)
            {
                var primary = new RustinoSingleInstance(appId, SingleInstanceStatus.Primary, endpoint, options, arguments, lockFile);
                try
                {
                    primary.StartServer();
                    return primary;
                }
                catch
                {
                    primary.Dispose();
                    throw;
                }
            }

            if (!options.ForwardAutomatically)
                return new RustinoSingleInstance(appId, SingleInstanceStatus.Secondary, endpoint, options, arguments, null);

            var remaining = deadline - DateTime.UtcNow;
            if (remaining <= TimeSpan.Zero)
                return new RustinoSingleInstance(appId, SingleInstanceStatus.ForwardFailed, endpoint, options, arguments, null);

            var result = SingleInstanceClient.TryForwardAsync(
                    endpoint,
                    CreateRequest(arguments),
                    Min(remaining, TimeSpan.FromMilliseconds(500)))
                .GetAwaiter().GetResult();
            if (result == ForwardResult.Delivered)
                return new RustinoSingleInstance(appId, SingleInstanceStatus.Forwarded, endpoint, options, arguments, null);
            if (result == ForwardResult.Rejected || DateTime.UtcNow >= deadline)
                return new RustinoSingleInstance(appId, SingleInstanceStatus.ForwardFailed, endpoint, options, arguments, null);

            var retryDelay = deadline - DateTime.UtcNow;
            if (retryDelay > TimeSpan.Zero)
                Thread.Sleep(Min(TimeSpan.FromMilliseconds(50), retryDelay));
        }
    }

    /// <summary>The app id passed to <see cref="Acquire"/>.</summary>
    public string AppId { get; }

    /// <summary>Whether this process owns the app id.</summary>
    public SingleInstanceStatus Status { get; }

    /// <summary>True when this process owns the app id.</summary>
    public bool IsPrimary => Status == SingleInstanceStatus.Primary;

    /// <summary>Raised sequentially on a background thread when a later process forwards its launch.</summary>
    public event EventHandler<SecondInstanceEventArgs>? SecondInstanceStarted
    {
        add
        {
            if (value is null) return;
            lock (_gate)
            {
                _secondInstanceStarted += value;
                StartDispatcherIfNeeded();
            }
        }
        remove { lock (_gate) _secondInstanceStarted -= value; }
    }

    /// <summary>Raised when a second-instance event handler throws.</summary>
    public event EventHandler<SingleInstanceExceptionEventArgs>? UnhandledException
    {
        add { lock (_gate) _unhandledException += value; }
        remove { lock (_gate) _unhandledException -= value; }
    }

    /// <summary>
    /// Optional window to activate after each forwarded launch. Setting it also begins dispatching any
    /// launches already received. Set event handlers first if they need to process those launches.
    /// </summary>
    public RustinoWindow? MainWindow
    {
        get { lock (_gate) return _mainWindow; }
        set
        {
            lock (_gate)
            {
                _mainWindow = value;
                StartDispatcherIfNeeded();
            }
        }
    }

    /// <summary>
    /// Forwards arguments to the primary process. Returns false if it could not acknowledge within the
    /// timeout. Throws when called by the primary process.
    /// </summary>
    public bool ForwardToPrimary(IReadOnlyList<string>? args = null, TimeSpan? timeout = null)
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) != 0, this);
        if (IsPrimary)
            throw new InvalidOperationException("The primary process cannot forward to itself.");

        var limit = timeout ?? _options.Timeout;
        ValidateTimeout(limit, nameof(timeout));
        return ForwardUntilAcknowledged(_endpoint, CreateRequest((args ?? _arguments).ToArray()), limit, _options.Logger)
            == ForwardResult.Delivered;
    }

    /// <summary>Stops receiving launches, waits for the active dispatcher, then releases the app id.</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
            return;

        try
        {
            if (_server is not null)
                _server.StopAsync().GetAwaiter().GetResult();
        }
        finally
        {
            // Stop the listener before completing the queue. Let an active dispatcher
            // handle requests already acknowledged by the server before releasing the lock.
            _messages.Writer.TryComplete();
            try
            {
                // An event handler may dispose its own instance. It cannot wait for the
                // dispatcher task whose current callback is this Dispose call.
                if (Volatile.Read(ref _dispatchThreadId) != Environment.CurrentManagedThreadId)
                    _dispatcher?.GetAwaiter().GetResult();
            }
            finally
            {
                _lockFile?.Dispose();
            }
        }
    }

    private void StartServer()
    {
        _server = new SingleInstanceServer(_endpoint, QueueMessage, _options.Logger);
        _server.Start();
    }

    private bool QueueMessage(SingleInstanceRequest request)
    {
        var message = new SecondInstanceEventArgs(
            request.Args,
            request.WorkingDirectory,
            request.ProcessId,
            request.ActivationToken,
            _options.ActivateMainWindow);
        return _messages.Writer.TryWrite(message);
    }

    private void StartDispatcherIfNeeded()
    {
        if (!IsPrimary || Volatile.Read(ref _disposed) != 0 || _dispatcher is not null
            || (_secondInstanceStarted is null && _mainWindow is null))
            return;
        _dispatcher = Task.Run(DispatchMessagesAsync);
    }

    private async Task DispatchMessagesAsync()
    {
        await foreach (var message in _messages.Reader.ReadAllAsync().ConfigureAwait(false))
        {
            Volatile.Write(ref _dispatchThreadId, Environment.CurrentManagedThreadId);
            try
            {
                EventHandler<SecondInstanceEventArgs>? handlers;
                lock (_gate) handlers = _secondInstanceStarted;
                if (handlers is not null)
                {
                    foreach (EventHandler<SecondInstanceEventArgs> handler in handlers.GetInvocationList())
                    {
                        try { handler(this, message); }
                        catch (Exception exception) { ReportHandlerException(exception, handler.Method.Name); }
                    }
                }

                if (message.ActivateMainWindow)
                {
                    RustinoWindow? window;
                    lock (_gate) window = _mainWindow;
                    try { window?.Activate(message.ActivationToken); }
                    catch (Exception exception) { ReportHandlerException(exception, nameof(MainWindow)); }
                }
            }
            finally
            {
                Volatile.Write(ref _dispatchThreadId, 0);
            }
        }
    }

    private void ReportHandlerException(Exception exception, string handlerName)
    {
        _options.Logger?.LogError(exception, "Single-instance handler {HandlerName} failed.", handlerName);
        EventHandler<SingleInstanceExceptionEventArgs>? handlers;
        lock (_gate) handlers = _unhandledException;
        if (handlers is null) return;
        var args = new SingleInstanceExceptionEventArgs(exception, handlerName);
        foreach (EventHandler<SingleInstanceExceptionEventArgs> handler in handlers.GetInvocationList())
        {
            try { handler(this, args); }
            catch (Exception reportException)
            {
                _options.Logger?.LogError(reportException, "Single-instance exception reporting handler failed.");
            }
        }
    }

    private static SingleInstanceRequest CreateRequest(string[] args) => new(
        args,
        Environment.CurrentDirectory,
        Environment.ProcessId,
        OperatingSystem.IsLinux()
            ? Environment.GetEnvironmentVariable("XDG_ACTIVATION_TOKEN")
                ?? Environment.GetEnvironmentVariable("DESKTOP_STARTUP_ID")
            : null);

    private static ForwardResult ForwardUntilAcknowledged(
        SingleInstanceEndpoint endpoint,
        SingleInstanceRequest request,
        TimeSpan timeout,
        ILogger? logger)
    {
        var deadline = DateTime.UtcNow + timeout;
        do
        {
            var remaining = deadline - DateTime.UtcNow;
            if (remaining <= TimeSpan.Zero) break;
            var result = SingleInstanceClient.TryForwardAsync(endpoint, request, Min(remaining, TimeSpan.FromMilliseconds(500)))
                .GetAwaiter().GetResult();
            if (result != ForwardResult.Unavailable)
                return result;
            var delay = deadline - DateTime.UtcNow;
            if (delay > TimeSpan.Zero)
                Thread.Sleep(Min(TimeSpan.FromMilliseconds(50), delay));
        } while (DateTime.UtcNow < deadline);

        logger?.LogWarning("No single-instance primary acknowledged app id {AppId} within {Timeout}.", endpoint.DisplayId, timeout);
        return ForwardResult.Unavailable;
    }

    private static TimeSpan Min(TimeSpan left, TimeSpan right) => left < right ? left : right;

    private static void ValidateTimeout(TimeSpan timeout, string parameterName)
    {
        if (timeout <= TimeSpan.Zero || timeout > TimeSpan.FromMinutes(5))
            throw new ArgumentOutOfRangeException(parameterName, "Timeout must be greater than zero and no more than five minutes.");
    }
}

internal enum ForwardResult { Delivered, Rejected, Unavailable }

internal sealed record SingleInstanceRequest(string[] Args, string WorkingDirectory, int ProcessId, string? ActivationToken);
