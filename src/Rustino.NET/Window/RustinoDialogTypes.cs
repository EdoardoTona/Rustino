namespace Rustino.NET;

// The values of Photino's PhotinoDialogButtons, PhotinoDialogIcon and PhotinoDialogResult

public enum RustinoDialogButtons
{
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    RetryCancel,
    AbortRetryIgnore,
}

public enum RustinoDialogIcon
{
    Info,
    Warning,
    Error,
    /// <summary>Shown as <see cref="Info"/> on macOS.</summary>
    Question,
}

public enum RustinoDialogResult
{
    /// <summary>Also when the dialog is closed without choosing a button, e.g. with Escape.</summary>
    Cancel = -1,
    Ok,
    Yes,
    No,
    Abort,
    Retry,
    Ignore,
}
