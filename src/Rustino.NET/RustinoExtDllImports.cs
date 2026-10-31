using System.Runtime.InteropServices;

namespace Rustino.NET;

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void ScaleFactorCallback(IntPtr context, double scaleFactor, int width, int height);

// Native window features (window_ext.rs)
internal static partial class RustinoExtDllImports
{
    private const string Lib = NativeLibraryResolver.LibName;

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_theme(RustinoWindowHandle instance, int theme);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_get_theme(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern double rustino_get_scale_factor(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_progress_bar(RustinoWindowHandle instance, int state, int progress);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_request_user_attention(RustinoWindowHandle instance, int kind);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_beep(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_shadow(RustinoWindowHandle instance, int shadow);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_skip_taskbar(RustinoWindowHandle instance, int skip);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_content_protection(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_visible_on_all_workspaces(RustinoWindowHandle instance, int visible);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_closable(RustinoWindowHandle instance, int closable);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_minimizable(RustinoWindowHandle instance, int minimizable);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_maximizable(RustinoWindowHandle instance, int maximizable);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_always_on_bottom(RustinoWindowHandle instance, int onBottom);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_ignore_cursor_events(RustinoWindowHandle instance, int ignore);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_mac_title_bar_style(RustinoWindowHandle instance, int style);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_title_bar_overlay(RustinoWindowHandle instance, int overlay);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_title_bar_overlay_color(RustinoWindowHandle instance, byte r, byte g, byte b, byte a);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_drag_regions_enabled(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_traffic_light_position(RustinoWindowHandle instance, double x, double y);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_desktop_file_name(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_drag_window(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_drag_resize_window(RustinoWindowHandle instance, int direction);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_theme_changed_handler(RustinoWindowHandle instance, IntCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_scale_factor_changed_handler(RustinoWindowHandle instance, ScaleFactorCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_urls_opened_handler(RustinoWindowHandle instance, StringCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_reopen_handler(RustinoWindowHandle instance, IntCallback handler);
}
