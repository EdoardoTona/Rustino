//! The menu bar of a window (on macOS, of the app) and its context menus, on each platform.

use crate::config::WindowConfig;
use crate::icon;

use super::{BuiltMenu, MenuItems, MenuOwner};

/// Menu bar shown on macOS until `SetMenu`, after the standard application menu. Cmd+C/V/X/A/Z
/// reach the webview only through the Edit items.
#[cfg(any(target_os = "macos", test))]
pub(crate) const DEFAULT_MACOS_MENU: &str = r#"[
    {"type": "submenu", "label": "Edit", "items": [
        {"type": "predefined", "item": "undo"},
        {"type": "predefined", "item": "redo"},
        {"type": "separator"},
        {"type": "predefined", "item": "cut"},
        {"type": "predefined", "item": "copy"},
        {"type": "predefined", "item": "paste"},
        {"type": "predefined", "item": "select_all"}
    ]},
    {"type": "submenu", "label": "Window", "role": "window", "items": [
        {"type": "predefined", "item": "minimize"},
        {"type": "predefined", "item": "maximize"},
        {"type": "separator"},
        {"type": "predefined", "item": "bring_all_to_front"}
    ]}
]"#;

/// Replaces the window's menu bar (on macOS, the application's).
pub(crate) fn set_menu_bar(
    built: BuiltMenu,
    window: &tao::window::Window,
    _config: &WindowConfig,
    _about: &muda::AboutMetadata,
    current_menu: &mut Option<muda::Menu>,
    menu_items: &mut MenuItems,
) {
    if let Some(old) = current_menu.take() {
        remove_menu_from_window(&old, window);
    }
    // On macOS the first submenu of the menu bar becomes the application menu: without
    // AddAppMenu the standard one is prepended so the first custom submenu stays visible.
    #[cfg(target_os = "macos")]
    let app_menu = built
        .app_menu
        .or_else(|| Some(create_macos_app_menu(_config, _about)));
    #[cfg(not(target_os = "macos"))]
    let app_menu = built.app_menu;
    if let Some(app_menu) = app_menu {
        let _ = built.menu.prepend(&app_menu);
    }
    attach_menu_to_window(&built.menu, built.custom_edit_shortcuts, window);
    // After init_for_nsapp, as muda requires
    #[cfg(target_os = "macos")]
    {
        if let Some(window_menu) = &built.window_menu {
            window_menu.set_as_windows_menu_for_nsapp();
        }
        if let Some(help_menu) = &built.help_menu {
            help_menu.set_as_help_menu_for_nsapp();
        }
    }
    menu_items.replace(MenuOwner::MenuBar, built.items);
    *current_menu = Some(built.menu);
}

/// Standard macOS application menu: About, Hide, Hide Others, Show All, Quit.
#[cfg(target_os = "macos")]
pub(crate) fn create_macos_app_menu(
    config: &WindowConfig,
    about: &muda::AboutMetadata,
) -> muda::Submenu {
    let app_menu = muda::Submenu::new("App", true);

    // Without SetAboutName, muda and AppKit use the app name shown in the menu bar
    let name = config.about_name.as_deref();
    let label = |action: &str| name.map(|name| format!("{action} {name}"));
    let (about_label, hide, quit) = (label("About"), label("Hide"), label("Quit"));

    let _ = app_menu.append(&muda::PredefinedMenuItem::about(
        about_label.as_deref(),
        Some(about.clone()),
    ));
    let _ = app_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = app_menu.append(&muda::PredefinedMenuItem::hide(hide.as_deref()));
    let _ = app_menu.append(&muda::PredefinedMenuItem::hide_others(None));
    let _ = app_menu.append(&muda::PredefinedMenuItem::show_all(None));
    let _ = app_menu.append(&muda::PredefinedMenuItem::separator());
    let _ = app_menu.append(&muda::PredefinedMenuItem::quit(quit.as_deref()));

    app_menu
}

/// Metadata for the predefined About item, from the SetAbout* values and the window icon.
pub(crate) fn about_metadata(config: &WindowConfig) -> muda::AboutMetadata {
    let authors = (!config.about_authors.is_empty()).then(|| config.about_authors.clone());
    // macOS's About panel ignores comments, authors, website and license: show them as credits
    let credits = [
        config.about_comments.clone(),
        authors.as_ref().map(|a| a.join(", ")),
        config.about_website.clone(),
        config.about_license.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");

    muda::AboutMetadata {
        name: config.about_name.clone(),
        version: config.about_version.clone(),
        copyright: config.about_copyright.clone(),
        website: config.about_website.clone(),
        license: config.about_license.clone(),
        authors,
        comments: config.about_comments.clone(),
        credits: (!credits.is_empty()).then_some(credits),
        // Without an explicit icon, an unbundled app shows its folder icon
        icon: config.icon_file.as_deref().and_then(icon::load_menu_icon),
        ..Default::default()
    }
}

/// `_custom_edit_shortcuts`: see `BuiltMenu::custom_edit_shortcuts` (Windows only).
fn attach_menu_to_window(menu: &muda::Menu, _custom_edit_shortcuts: u8, _window: &tao::window::Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe { let _ = menu.init_for_hwnd(_window.hwnd() as _); }
        super::accelerators::set_menu(_window.hwnd(), Some(menu), _custom_edit_shortcuts);
    }
    #[cfg(target_os = "macos")]
    {
        menu.init_for_nsapp();
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        // The GTK theme draws the menu bar transparent, so a background color set on the window
        // (e.g. a dark one) shows through while the labels keep the theme's text color: on a
        // light theme that is dark text over a dark background. Give the bar the theme's own
        // background so background and text stay readable and consistent with the title bar.
        // Applied regardless of the window background: with most themes it changes nothing,
        // but it makes the bar opaque on a transparent window.
        // Only on success: a failed init would leave no bar of this menu to style. The method
        // takes the menu by value, and cloning a muda::Menu only clones its handle.
        let menu_bar = menu
            .init_for_gtk_window(_window.gtk_window(), _window.default_vbox())
            .ok()
            .and_then(|_| menu.clone().gtk_menubar_for_gtk_window(_window.gtk_window()));
        if let Some(menu_bar) = menu_bar {
            let provider = gtk::CssProvider::new();
            let _ = provider.load_from_data(
                b"menubar { background-color: @theme_bg_color; color: @theme_fg_color; }",
            );
            menu_bar
                .style_context()
                .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    }
}

pub(crate) fn remove_menu_from_window(menu: &muda::Menu, _window: &tao::window::Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe { let _ = menu.remove_for_hwnd(_window.hwnd() as _); }
        super::accelerators::set_menu(_window.hwnd(), None, 0);
    }
    #[cfg(target_os = "macos")]
    {
        menu.remove_for_nsapp();
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu.remove_for_gtk_window(_window.gtk_window());
    }
}

pub(crate) fn show_context_menu(
    menu: &muda::Menu,
    window: &tao::window::Window,
    pos: Option<(f64, f64)>,
) {
    use muda::ContextMenu;
    let position = pos.map(|(x, y)| muda::dpi::Position::Physical(muda::dpi::PhysicalPosition::new(x as i32, y as i32)));
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        let _ = unsafe { menu.show_context_menu_for_hwnd(window.hwnd() as _, position) };
    }
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowExtMacOS;
        let _ = unsafe { menu.show_context_menu_for_nsview(window.ns_view() as _, position) };
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        use gtk::prelude::Cast;
        let _ = menu.show_context_menu_for_gtk_window(window.gtk_window().upcast_ref::<gtk::Window>(), position);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_macos_menu_parses() {
        let defs: Vec<crate::menu::MenuItemDef> =
            serde_json::from_str(super::DEFAULT_MACOS_MENU).unwrap();
        assert_eq!(defs.len(), 2, "Edit and Window submenus");
    }
}
