using Microsoft.Extensions.Logging;

namespace Rustino.NET;

/// <summary>Result of acquiring or forwarding to a single app instance.</summary>
public enum SingleInstanceStatus
{
    /// <summary>This process owns the app id and receives launches from later processes.</summary>
    Primary,
    /// <summary>The primary acknowledged this process's forwarded arguments.</summary>
    Forwarded,
    /// <summary>A primary owns the app id but did not acknowledge this process.</summary>
    ForwardFailed,
    /// <summary>Another process owns the app id; automatic forwarding was disabled.</summary>
    Secondary,
}

/// <summary>Options for <see cref="RustinoSingleInstance.Acquire"/>.</summary>
public sealed class SingleInstanceOptions
{
    /// <summary>Arguments to send when this process is secondary. Defaults to arguments passed to Main.</summary>
    public IReadOnlyList<string>? Arguments { get; init; }

    /// <summary>Automatically send arguments to the primary while acquiring. Defaults to true.</summary>
    public bool ForwardAutomatically { get; init; } = true;

    /// <summary>Maximum time to find a primary that is still starting. Defaults to five seconds.</summary>
    public TimeSpan Timeout { get; init; } = TimeSpan.FromSeconds(5);

    /// <summary>Whether each received launch activates <see cref="RustinoSingleInstance.MainWindow"/>.</summary>
    public bool ActivateMainWindow { get; init; } = true;

    /// <summary>Receives listener and handler failures.</summary>
    public ILogger? Logger { get; init; }
}

/// <summary>Arguments and launch context forwarded from another process.</summary>
public sealed class SecondInstanceEventArgs : EventArgs
{
    internal SecondInstanceEventArgs(string[] args, string workingDirectory, int processId, string? activationToken, bool activateMainWindow)
    {
        Args = args;
        WorkingDirectory = workingDirectory;
        ProcessId = processId;
        ActivationToken = activationToken;
        ActivateMainWindow = activateMainWindow;
    }

    /// <summary>Arguments supplied to the secondary process's Main method.</summary>
    public string[] Args { get; }

    /// <summary>Working directory of the secondary process.</summary>
    public string WorkingDirectory { get; }

    /// <summary>Process id of the secondary process.</summary>
    public int ProcessId { get; }

    /// <summary>Linux desktop activation token, when the launcher supplied one.</summary>
    public string? ActivationToken { get; }

    /// <summary>Set false in a handler to suppress activation of the configured main window.</summary>
    public bool ActivateMainWindow { get; set; }
}

/// <summary>An exception thrown by a single-instance event handler.</summary>
public sealed class SingleInstanceExceptionEventArgs(Exception exception, string handlerName) : EventArgs
{
    public Exception Exception { get; } = exception;
    public string HandlerName { get; } = handlerName;
}
