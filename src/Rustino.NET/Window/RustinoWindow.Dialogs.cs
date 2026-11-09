namespace Rustino.NET;

public partial class RustinoWindow
{
    // --- Dialogs (post-run) ---
    // While the window runs, dialogs are modal for it (sheets on macOS) and can be called from any thread.

    public string[]? ShowOpenFileDialog(
        string? title = null,
        string? defaultPath = null,
        FileFilter[]? filters = null,
        bool multiSelect = false)
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        var filterStr = FileFilter.Encode(filters);
        var ptr = RustinoDllImports.rustino_show_open_file_dialog(
            handle, title, defaultPath, filterStr, multiSelect ? 1 : 0);
        return ConsumePathResult(ptr);
    }

    public string? ShowSaveFileDialog(
        string? title = null,
        string? defaultPath = null,
        FileFilter[]? filters = null)
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        var filterStr = FileFilter.Encode(filters);
        var ptr = RustinoDllImports.rustino_show_save_file_dialog(
            handle, title, defaultPath, filterStr);
        return ConsumeStringResult(ptr);
    }

    public string[]? ShowSelectFolderDialog(
        string? title = null,
        string? defaultPath = null,
        bool multiSelect = false)
    {
        ThrowIfDisposed();
        if (Native is not { } handle) return null;
        var ptr = RustinoDllImports.rustino_show_select_folder_dialog(
            handle, title, defaultPath, multiSelect ? 1 : 0);
        return ConsumePathResult(ptr);
    }

    // Photino's ShowMessage. Before the window runs (or after it closes) the message box has no parent.
    public RustinoDialogResult ShowMessage(
        string title,
        string text,
        RustinoDialogButtons buttons = RustinoDialogButtons.Ok,
        RustinoDialogIcon icon = RustinoDialogIcon.Info)
    {
        ThrowIfDisposed();
        // The parent is optional, so it's passed as a pointer: keep the window alive by hand
        var parent = Native;
        var addedRef = false;
        try
        {
            parent?.DangerousAddRef(ref addedRef);
            return (RustinoDialogResult)RustinoDllImports.rustino_show_message(
                addedRef ? parent!.DangerousGetHandle() : IntPtr.Zero, title, text, (int)buttons, (int)icon);
        }
        finally
        {
            if (addedRef)
                parent!.DangerousRelease();
        }
    }
}
