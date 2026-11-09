namespace Rustino.NET;

// Menu bar, context menus, tray icon and taskbar badge.
public partial class RustinoWindow
{
    // --- Menus (post-run) ---

    public RustinoWindow SetMenu(RustinoMenu menu)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu(handle, menu.ToJson());
        return this;
    }

    public RustinoWindow RemoveMenu()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_remove_menu(handle);
        return this;
    }

    public RustinoWindow ShowContextMenu(RustinoMenu menu, double? x = null, double? y = null)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_show_context_menu(
                handle, menu.ToJson(), x ?? -1, y ?? -1);
        return this;
    }

    // --- Menu items (post-run) ---
    // Changes apply to every item with this id in the menu bar and in the tray menu. Context menus
    // are built from their RustinoMenu at every ShowContextMenu.

    public RustinoWindow SetMenuItemEnabled(string id, bool enabled)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu_item_enabled(handle, id, enabled ? 1 : 0);
        return this;
    }

    public RustinoWindow SetMenuItemChecked(string id, bool isChecked)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu_item_checked(handle, id, isChecked ? 1 : 0);
        return this;
    }

    public RustinoWindow SetMenuItemText(string id, string text)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(text);
        if (Native is { } handle)
            RustinoDllImports.rustino_set_menu_item_text(handle, id, text);
        return this;
    }

    // --- System Tray (post-run) ---

    // tooltip: also the icon's accessible name, defaulting to the window title (ignored on Linux,
    // where only title identifies the icon).
    // title: text next to the icon (macOS, Linux). isTemplateIcon (macOS): the icon's alpha is used
    // as a mask that follows the menu bar's colors. menuOnLeftClick: false leaves left clicks to
    // TrayIconClicked (macOS, Windows).
    public RustinoWindow SetTrayIcon(string iconPath, string? tooltip = null, RustinoMenu? menu = null,
        string? title = null, bool isTemplateIcon = false, bool menuOnLeftClick = true) =>
        SetTrayIcon(iconPath, tempFile: false, tooltip, menu, title, isTemplateIcon, menuOnLeftClick);

    // Written to a temporary file, deleted when the tray icon is replaced or removed and by Dispose
    public RustinoWindow SetTrayIcon(Stream icon, string? tooltip = null, RustinoMenu? menu = null,
        string? title = null, bool isTemplateIcon = false, bool menuOnLeftClick = true)
    {
        ThrowIfDisposed();
        return SetTrayIcon(WriteTempFile(icon, "rustino_tray"), tempFile: true,
            tooltip, menu, title, isTemplateIcon, menuOnLeftClick);
    }

    private RustinoWindow SetTrayIcon(string iconPath, bool tempFile, string? tooltip, RustinoMenu? menu,
        string? title, bool isTemplateIcon, bool menuOnLeftClick)
    {
        var used = false;
        try
        {
            ThrowIfDisposed();
            if (Native is { } handle)
            {
                RustinoDllImports.rustino_set_tray_icon(
                    handle, iconPath, tooltip, menu?.ToJson(), title,
                    isTemplateIcon ? 1 : 0, menuOnLeftClick ? 1 : 0);
                used = true;
            }
        }
        finally
        {
            if (tempFile && !used) DeleteFile(iconPath);
        }
        DeleteFile(Interlocked.Exchange(ref _trayIconTempFile, used && tempFile ? iconPath : null));
        return this;
    }

    // Text next to the tray icon (macOS, Linux); null removes it.
    public RustinoWindow SetTrayTitle(string? title)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_set_tray_title(handle, title);
        return this;
    }

    // --- Badge ---

    public RustinoWindow SetBadgeCount(int? count, string? background = null, string? foreground = null)
    {
        ThrowIfDisposed();
        if (Native is { } handle)
        {
            var (bgR, bgG, bgB) = ParseHexColor(background, 0xE0, 0x1E, 0x5A);
            var (fgR, fgG, fgB) = ParseHexColor(foreground, 0xFF, 0xFF, 0xFF);
            RustinoDllImports.rustino_set_badge_count(handle, count ?? 0, bgR, bgG, bgB, fgR, fgG, fgB);
        }
        return this;
    }

    public RustinoWindow ClearBadge()
    {
        ThrowIfDisposed();
        return SetBadgeCount(null);
    }

    private static (byte r, byte g, byte b) ParseHexColor(string? hex, byte defaultR, byte defaultG, byte defaultB)
    {
        if (string.IsNullOrEmpty(hex))
            return (defaultR, defaultG, defaultB);
        var s = hex.StartsWith('#') ? hex[1..] : hex;
        if (s.Length == 6 &&
            byte.TryParse(s[0..2], System.Globalization.NumberStyles.HexNumber, null, out var r) &&
            byte.TryParse(s[2..4], System.Globalization.NumberStyles.HexNumber, null, out var g) &&
            byte.TryParse(s[4..6], System.Globalization.NumberStyles.HexNumber, null, out var b))
        {
            return (r, g, b);
        }
        return (defaultR, defaultG, defaultB);
    }

    public RustinoWindow RemoveTrayIcon()
    {
        ThrowIfDisposed();
        if (Native is { } handle)
            RustinoDllImports.rustino_remove_tray_icon(handle);
        DeleteFile(Interlocked.Exchange(ref _trayIconTempFile, null));
        return this;
    }
}
