using System.Runtime.InteropServices;

namespace Rustino.NET;

// Callback delegate types matching Rust extern "C" fn signatures
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int ClosingCallback(IntPtr context);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void VoidContextCallback(IntPtr context);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void SizeCallback(IntPtr context, int width, int height);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void PointCallback(IntPtr context, int x, int y);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void IntCallback(IntPtr context, int value);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void StringCallback(IntPtr context, IntPtr message);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void WebMessageCallback(IntPtr context, IntPtr message, IntPtr sourceUrl);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void PageLoadCallback(IntPtr context, int eventType, IntPtr url);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int NavigationCallback(IntPtr context, IntPtr url);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void CustomSchemeCallback(IntPtr context, IntPtr url, IntPtr response);

// isChecked: new state of a check item (0/1), -1 for other items
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void MenuItemCallback(IntPtr context, IntPtr id, int isChecked);

// button: 0 left, 1 right, 2 middle
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void TrayIconCallback(IntPtr context, int button, int x, int y);

internal static partial class RustinoDllImports
{
    private const string Lib = NativeLibraryResolver.LibName;

    // --- Lifecycle ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_ctor(ref RustinoNativeParameters parameters);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_dtor(IntPtr instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_wait_for_exit(RustinoWindowHandle instance, out IntPtr error);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_close(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_free_string(IntPtr s);

    // --- Notifications ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_show_notification(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string title,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string body,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? icon,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? appId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_register_notification_app_id(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string appId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? displayName,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? iconPath);

    // --- Dual-mode setters ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_title(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string title);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_size(RustinoWindowHandle instance, int width, int height);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_resizable(RustinoWindowHandle instance, int resizable);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_topmost(RustinoWindowHandle instance, int topmost);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_icon_file(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_center(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_about_name(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_about_version(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string version);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_about_copyright(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string copyright);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_about_website(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string website);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_about_license(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string license);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_about_authors(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string authors);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_about_comments(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string comments);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_navigate_to_url(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string url);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_navigate_to_string(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string content);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_background_color(
        RustinoWindowHandle instance, byte r, byte g, byte b, byte a);

    // --- Pre-run only setters ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_use_os_default_size(RustinoWindowHandle instance, int useDefault);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_devtools_enabled(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_clipboard_enabled(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_ignore_cert_errors(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_web_security_enabled(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_transparent(RustinoWindowHandle instance, int transparent);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_decorations(RustinoWindowHandle instance, int decorated);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_user_agent(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? userAgent);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_user_data_folder(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? path);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_media_autoplay(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_set_zoom_hotkeys(RustinoWindowHandle instance, int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_add_init_script(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string js);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_add_custom_scheme(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string scheme);

    // --- Custom scheme response (called from within the custom scheme callback) ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_scheme_response(
        IntPtr response,
        byte[] data,
        int length,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? contentType);

    // --- Window state ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_minimized(RustinoWindowHandle instance, int minimized);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_maximized(RustinoWindowHandle instance, int maximized);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_fullscreen(RustinoWindowHandle instance, int fullscreen);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_visible(RustinoWindowHandle instance, int visible);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_focus(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_position(RustinoWindowHandle instance, int x, int y);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_min_size(RustinoWindowHandle instance, int width, int height);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_max_size(RustinoWindowHandle instance, int width, int height);

    // --- State queries ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_is_minimized(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_is_maximized(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_is_fullscreen(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_get_position(RustinoWindowHandle instance, out int x, out int y);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_get_size(RustinoWindowHandle instance, out int width, out int height);

    // --- WebView operations ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_evaluate_script(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string js);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_send_web_message(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string message);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_zoom(RustinoWindowHandle instance, double factor);

    // --- Dialogs ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_show_open_file_dialog(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? title,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? defaultPath,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? filters,
        int multiSelect);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_show_save_file_dialog(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? title,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? defaultPath,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? filters);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_show_select_folder_dialog(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? title,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? defaultPath,
        int multiSelect);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int rustino_show_message(
        IntPtr instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string title,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string text,
        int buttons,
        int icon);

    // --- Monitors ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_get_monitors(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_get_current_monitor(RustinoWindowHandle instance);

    // --- Badge ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_badge_count(
        RustinoWindowHandle instance, int count,
        byte bgR, byte bgG, byte bgB,
        byte fgR, byte fgG, byte fgB);

    // --- Menus & Tray ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_menu(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string json);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_remove_menu(RustinoWindowHandle instance);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_show_context_menu(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string json,
        double x,
        double y);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_menu_item_enabled(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string id,
        int enabled);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_menu_item_checked(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string id,
        int isChecked);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_menu_item_text(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string text);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_tray_icon(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string iconPath,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? tooltip,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? menuJson,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? title,
        int isTemplateIcon,
        int menuOnLeftClick);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_tray_title(
        RustinoWindowHandle instance,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? title);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_remove_tray_icon(RustinoWindowHandle instance);

    // --- Callback registration ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_callback_context(RustinoWindowHandle instance, IntPtr context);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_closing_handler(RustinoWindowHandle instance, ClosingCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_closed_handler(RustinoWindowHandle instance, VoidContextCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_resized_handler(RustinoWindowHandle instance, SizeCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_moved_handler(RustinoWindowHandle instance, PointCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_focus_changed_handler(RustinoWindowHandle instance, IntCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_web_message_received_handler(RustinoWindowHandle instance, WebMessageCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_page_load_handler(RustinoWindowHandle instance, PageLoadCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_navigation_handler(RustinoWindowHandle instance, NavigationCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_menu_event_handler(RustinoWindowHandle instance, MenuItemCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_tray_icon_event_handler(RustinoWindowHandle instance, TrayIconCallback handler);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_set_custom_scheme_handler(RustinoWindowHandle instance, CustomSchemeCallback handler);

    // --- Splashscreen ---

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr rustino_splash_create(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string imagePath,
        int width,
        int height);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_splash_close(IntPtr splash);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void rustino_splash_dtor(IntPtr splash);
}
