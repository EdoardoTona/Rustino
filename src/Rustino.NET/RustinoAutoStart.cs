namespace Rustino.NET;

/// <summary>State of the start at login entry of an app.</summary>
public enum AutoStartStatus
{
    /// <summary>The platform has no start at login for this app (e.g. a Flatpak sandbox).</summary>
    Unsupported,
    /// <summary>The app doesn't start at login.</summary>
    Disabled,
    /// <summary>The app starts at login.</summary>
    Enabled,
    /// <summary>
    /// The app asked to start at login, but the user turned it off from the system: Task Manager or
    /// Settings › Apps › Startup on Windows, System Settings › Login Items on macOS, the startup applications
    /// of GNOME or KDE on Linux.
    /// </summary>
    DisabledBySystem,
}

/// <summary>An exception thrown by a <see cref="RustinoAutoStart.StatusChanged"/> handler or observer.</summary>
public sealed class AutoStartExceptionEventArgs(Exception exception) : EventArgs
{
    public Exception Exception { get; } = exception;
}

/// <summary>Options for <see cref="RustinoAutoStart"/>.</summary>
public sealed class AutoStartOptions
{
    /// <summary>Linux: the name in the startup applications. Defaults to the app id.</summary>
    public string? DisplayName { get; init; }

    /// <summary>
    /// The program started at login. Defaults to the running executable (with <c>dotnet app.dll</c>, the dotnet
    /// host and the app's assembly).
    /// </summary>
    public string? ExecutablePath { get; init; }

    /// <summary>Arguments passed at login, before <see cref="RustinoAutoStart.LaunchArgument"/>.</summary>
    public IReadOnlyList<string>? Arguments { get; init; }

    /// <summary>
    /// How often <see cref="RustinoAutoStart.StatusChanged"/> checks the system once someone listens. Defaults
    /// to two seconds.
    /// </summary>
    public TimeSpan PollInterval { get; init; } = TimeSpan.FromSeconds(2);
}

/// <summary>
/// Starts the app when the user logs in, for the current user only: a value of the <c>Run</c> registry key on
/// Windows, a launch agent in <c>~/Library/LaunchAgents</c> on macOS, an entry in <c>~/.config/autostart</c> on
/// Linux. The user can turn it off from the system too: <see cref="Status"/> and <see cref="StatusChanged"/>
/// tell it, so the app's settings show the real state.
/// </summary>
/// <remarks>
/// MSIX packaged apps can't use the <c>Run</c> key: they declare a <c>StartupTask</c> in their manifest.
/// </remarks>
public sealed class RustinoAutoStart : IDisposable
{
    /// <summary>
    /// Added to the arguments of the launch at login: <see cref="WasLaunchedAtLogin"/> looks for it.
    /// </summary>
    public const string LaunchArgument = "--autostart";

    private readonly object _gate = new();
    private readonly string _command;
    private readonly string? _displayName;
    private readonly TimeSpan _pollInterval;
    private readonly EventObservable<AutoStartStatus> _statusChangedObs = new();
    private EventHandler<AutoStartStatus>? _statusChanged;
    private EventHandler<AutoStartExceptionEventArgs>? _unhandledException;
    private AutoStartStatus? _lastStatus;
    private Timer? _timer;
    private bool _disposed;

    // The native library may load before any window: from runtimes/<rid>/native too
    static RustinoAutoStart() => NativeLibraryResolver.EnsureRegistered();

    /// <param name="appId">
    /// Names the entry: letters, digits, '.', '-' and '_', such as <c>com.example.MyApp</c>. The same id finds the
    /// entry again in later runs.
    /// </param>
    public RustinoAutoStart(string appId, AutoStartOptions? options = null)
    {
        if (!IsValidAppId(appId))
            throw new ArgumentException(
                "The app id must have 1 to 128 letters, digits, '.', '-' or '_', and not start with '.'.", nameof(appId));
        options ??= new AutoStartOptions();
        if (options.PollInterval <= TimeSpan.Zero)
            throw new ArgumentOutOfRangeException(nameof(options), "PollInterval must be positive.");

        var command = options.ExecutablePath is { } executable ? [Path.GetFullPath(executable)] : DefaultCommand();
        command.AddRange(options.Arguments ?? []);
        command.Add(LaunchArgument);
        if (command.Any(argument => argument is null || argument.Contains('\n') || argument.Contains('\0')))
            throw new ArgumentException("Arguments can't be null or contain line breaks or NUL characters.", nameof(options));

        AppId = appId;
        _command = string.Join('\n', command);
        _displayName = options.DisplayName;
        _pollInterval = options.PollInterval;
    }

    /// <summary>The id of the entry.</summary>
    public string AppId { get; }

    // The program and its arguments, separated by '\n'
    internal string Command => _command;

    /// <summary>
    /// Whether the system started this process at login, from an entry of <see cref="Enable"/>: the app can then
    /// stay in the tray instead of showing its window.
    /// </summary>
    public static bool WasLaunchedAtLogin => Environment.GetCommandLineArgs().Skip(1).Contains(LaunchArgument);

    /// <summary>The state of the entry, read from the system on each call.</summary>
    public AutoStartStatus Status
    {
        get
        {
            ThrowIfDisposed();
            return ToStatus(RustinoDllImports.rustino_autostart_status(AppId));
        }
    }

    /// <summary>
    /// The state of the entry changed, from <see cref="Enable"/>, <see cref="Disable"/> or the system. While someone
    /// listens the system is checked every <see cref="AutoStartOptions.PollInterval"/>; the event comes on a thread
    /// pool thread.
    /// </summary>
    public event EventHandler<AutoStartStatus>? StatusChanged
    {
        add
        {
            lock (_gate) _statusChanged += value;
            StartPolling();
        }
        remove
        {
            lock (_gate) _statusChanged -= value;
        }
    }

    /// <summary>
    /// A <see cref="StatusChanged"/> handler or a <see cref="WhenStatusChanged"/> observer threw: the exception
    /// doesn't reach the polling thread, which would end the process, nor <see cref="Enable"/> and
    /// <see cref="Disable"/>. Exceptions thrown here are dropped.
    /// </summary>
    public event EventHandler<AutoStartExceptionEventArgs>? UnhandledException
    {
        add { lock (_gate) _unhandledException += value; }
        remove { lock (_gate) _unhandledException -= value; }
    }

    /// <summary>The values of <see cref="StatusChanged"/>; subscribing starts the checks too.</summary>
    public IObservable<AutoStartStatus> WhenStatusChanged => new PollingObservable(this);

    /// <summary>
    /// Starts the app at login, also when the entry is there already (it points to the current executable again).
    /// After the user turned it off from the system, Windows and Linux turn it on again, as the user asked from
    /// the app; macOS keeps the choice of Login Items, and the status stays <see cref="AutoStartStatus.DisabledBySystem"/>:
    /// offer <see cref="OpenSystemSettings"/> then.
    /// </summary>
    /// <returns>The new state.</returns>
    /// <exception cref="RustinoException">The entry couldn't be written.</exception>
    public AutoStartStatus Enable()
    {
        ThrowIfDisposed();
        var status = RustinoDllImports.rustino_autostart_enable(AppId, _displayName, _command);
        if (status < 0)
            throw new RustinoException($"Could not write the start at login entry of '{AppId}'.");
        return Update(ToStatus(status));
    }

    /// <summary>The app no longer starts at login.</summary>
    /// <returns>The new state.</returns>
    /// <exception cref="RustinoException">The entry couldn't be removed.</exception>
    public AutoStartStatus Disable()
    {
        ThrowIfDisposed();
        var status = RustinoDllImports.rustino_autostart_disable(AppId);
        if (status < 0)
            throw new RustinoException($"Could not remove the start at login entry of '{AppId}'.");
        return Update(ToStatus(status));
    }

    /// <summary>
    /// Opens the system page of the startup apps: Settings › Apps › Startup on Windows, System Settings › Login
    /// Items on macOS. False on Linux, whose desktops have no common page.
    /// </summary>
    public static bool OpenSystemSettings() => RustinoDllImports.rustino_autostart_open_settings() != 0;

    /// <summary>Stops the checks and completes <see cref="WhenStatusChanged"/>. The entry stays.</summary>
    public void Dispose()
    {
        Timer? timer;
        lock (_gate)
        {
            if (_disposed) return;
            _disposed = true;
            timer = _timer;
            _timer = null;
            _statusChanged = null;
            _unhandledException = null;
        }
        timer?.Dispose();
        _statusChangedObs.Complete();
    }

    internal static bool IsValidAppId(string? appId) =>
        appId is { Length: > 0 and <= 128 }
        && appId[0] != '.'
        && appId.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '-' or '_');

    // `dotnet app.dll` runs the host: the entry needs the assembly too
    private static List<string> DefaultCommand()
    {
        var process = Environment.ProcessPath
            ?? throw new RustinoException("The path of the running executable is unknown: set AutoStartOptions.ExecutablePath.");
        var command = new List<string> { process };
        if (Path.GetFileNameWithoutExtension(process).Equals("dotnet", StringComparison.OrdinalIgnoreCase)
            && Environment.GetCommandLineArgs() is [var assembly, ..]
            && assembly.EndsWith(".dll", StringComparison.OrdinalIgnoreCase))
            command.Add(Path.GetFullPath(assembly));
        return command;
    }

    private static AutoStartStatus ToStatus(int status) =>
        Enum.IsDefined((AutoStartStatus)status) ? (AutoStartStatus)status : AutoStartStatus.Unsupported;

    private void StartPolling()
    {
        lock (_gate)
        {
            if (_disposed || _timer != null) return;
            _lastStatus ??= ToStatus(RustinoDllImports.rustino_autostart_status(AppId));
            _timer = new Timer(_ => Poll(), null, _pollInterval, _pollInterval);
        }
    }

    private void Poll()
    {
        lock (_gate)
            if (_disposed) return;
        try { Update(ToStatus(RustinoDllImports.rustino_autostart_status(AppId))); }
        catch (Exception exception) { ReportException(exception); }
    }

    internal AutoStartStatus Update(AutoStartStatus status)
    {
        EventHandler<AutoStartStatus>? handlers;
        lock (_gate)
        {
            if (_lastStatus == status) return status;
            // Before anyone listens there's nothing to compare with
            var changed = _lastStatus != null;
            _lastStatus = status;
            if (!changed || _disposed) return status;
            handlers = _statusChanged;
        }
        // Every handler and observer gets the status even when one throws
        if (handlers != null)
            foreach (EventHandler<AutoStartStatus> handler in handlers.GetInvocationList())
            {
                try { handler(this, status); }
                catch (Exception exception) { ReportException(exception); }
            }
        try { _statusChangedObs.Emit(status); }
        catch (Exception exception) { ReportException(exception); }
        return status;
    }

    private void ReportException(Exception exception)
    {
        EventHandler<AutoStartExceptionEventArgs>? handlers;
        lock (_gate) handlers = _unhandledException;
        if (handlers is null) return;
        var args = new AutoStartExceptionEventArgs(exception);
        foreach (EventHandler<AutoStartExceptionEventArgs> handler in handlers.GetInvocationList())
        {
            try { handler(this, args); }
            catch { /* nowhere left to report it */ }
        }
    }

    private void ThrowIfDisposed() => ObjectDisposedException.ThrowIf(_disposed, this);

    private sealed class PollingObservable(RustinoAutoStart owner) : IObservable<AutoStartStatus>
    {
        public IDisposable Subscribe(IObserver<AutoStartStatus> observer)
        {
            var subscription = owner._statusChangedObs.Subscribe(observer);
            owner.StartPolling();
            return subscription;
        }
    }
}
