using System.Runtime.InteropServices;

namespace Rustino.NET;

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void ScaleFactorCallback(IntPtr context, double scaleFactor, int width, int height);

// Native window features (window_ext.rs)
internal static partial class RustinoExtDllImports
{
    private const string Lib = NativeLibraryResolver.LibName;

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_theme(IntPtr instance, int theme);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_get_theme(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern double rustino_get_scale_factor(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_progress_bar(IntPtr instance, int state, int progress);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_request_user_attention(IntPtr instance, int kind);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_beep(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_shadow(IntPtr instance, int shadow);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_skip_taskbar(IntPtr instance, int skip);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_content_protection(IntPtr instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_visible_on_all_workspaces(IntPtr instance, int visible);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_closable(IntPtr instance, int closable);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_minimizable(IntPtr instance, int minimizable);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_maximizable(IntPtr instance, int maximizable);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_always_on_bottom(IntPtr instance, int onBottom);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_ignore_cursor_events(IntPtr instance, int ignore);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_mac_title_bar_style(IntPtr instance, int style);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_drag_regions_enabled(IntPtr instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_traffic_light_position(IntPtr instance, double x, double y);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_desktop_file_name(
        IntPtr instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_drag_window(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_drag_resize_window(IntPtr instance, int direction);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_theme_changed_handler(IntPtr instance, IntCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_scale_factor_changed_handler(IntPtr instance, ScaleFactorCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_urls_opened_handler(IntPtr instance, StringCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_reopen_handler(IntPtr instance, IntCallback handler);
}
