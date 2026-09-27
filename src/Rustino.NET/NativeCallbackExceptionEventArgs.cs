namespace Rustino.NET;

/// <summary>Information about an exception caught inside a callback invoked by Rustino.Native.</summary>
public sealed class NativeCallbackExceptionEventArgs(string callbackName, Exception exception) : EventArgs
{
    /// <summary>The managed callback in which the exception was caught.</summary>
    public string CallbackName { get; } = callbackName;

    /// <summary>The exception thrown by an event handler, observable observer, or logger.</summary>
    public Exception Exception { get; } = exception;
}
