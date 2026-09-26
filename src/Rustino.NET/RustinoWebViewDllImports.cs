using System.Runtime.InteropServices;

namespace Rustino.NET;

// kind: 0 enter, 1 over, 2 drop, 3 leave; paths separated by '\n'
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void FileDropCallback(IntPtr context, int kind, IntPtr paths, int x, int y);

// Returns 0 to cancel the download
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int DownloadStartingCallback(IntPtr context, IntPtr url, IntPtr suggestedPath, IntPtr response);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void DownloadCompletedCallback(IntPtr context, IntPtr url, IntPtr path, int success);

// Webview features (webview_ext.rs)
internal static class RustinoWebViewDllImports
{
    private const string Lib = NativeLibraryResolver.LibName;

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_context_menu_enabled(IntPtr instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_browser_accelerator_keys_enabled(IntPtr instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_scroll_bar_style(IntPtr instance, int style);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_accept_first_mouse(IntPtr instance, int accept);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_back_forward_gestures_enabled(IntPtr instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_file_drop_enabled(IntPtr instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_print(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_reload(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_open_devtools(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_close_devtools(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_clear_browsing_data(IntPtr instance);

    // status: 0 done, 1 not running, 2 called within a webview event on Windows
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_get_cookies(
        IntPtr instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? url,
        out int status);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_cookie(
        IntPtr instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string json);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_delete_cookie(
        IntPtr instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string json);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_file_drop_handler(IntPtr instance, FileDropCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_document_title_changed_handler(IntPtr instance, StringCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_download_starting_handler(IntPtr instance, DownloadStartingCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_download_completed_handler(IntPtr instance, DownloadCompletedCallback handler);

    // Called from within the download starting callback
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_download_destination(
        IntPtr response,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);
}
