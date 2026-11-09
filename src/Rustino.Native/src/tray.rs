//! The icon of the app in the system tray (the menu bar on macOS), with its menu.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;

use crate::commands::{RustinoCommand, TrayParams};
use crate::config::WindowConfig;
use crate::menu;
use crate::util;
use crate::window::{RustinoWindow, configure};

// Queued until the window runs

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_tray_icon(
    instance: *mut RustinoWindow,
    icon_path: *const c_char,
    tooltip: *const c_char,
    menu_json: *const c_char,
    title: *const c_char,
    icon_is_template: i32,
    menu_on_left_click: i32,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(path) = unsafe { util::cstr_to_string(icon_path) } {
                let params = TrayParams {
                    icon_path: path,
                    tooltip: unsafe { util::cstr_to_string(tooltip) },
                    menu_json: unsafe { util::cstr_to_string(menu_json) },
                    title: unsafe { util::cstr_to_string(title) },
                    icon_is_template: icon_is_template != 0,
                    menu_on_left_click: menu_on_left_click != 0,
                };
                inst.send_command(RustinoCommand::SetTrayIcon(params));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_tray_title(instance: *mut RustinoWindow, title: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::SetTrayTitle(unsafe { util::cstr_to_string(title) }));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_remove_tray_icon(instance: *mut RustinoWindow) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::RemoveTrayIcon);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_tray_icon_event_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32, i32, i32)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_tray_icon_clicked = handler) };
}

/// The tray icon of a window.
pub(crate) struct Tray {
    pub(crate) icon: tray_icon::TrayIcon,
    has_menu: bool,
    menu_on_left_click: bool,
}

impl Tray {
    /// Builds the tray icon with its menu, whose items join `menu_items`. `None` when the icon
    /// can't be loaded or shown.
    pub(crate) fn build(
        params: &TrayParams,
        window: &tao::window::Window,
        config: &WindowConfig,
        menu_items: &mut menu::MenuItems,
    ) -> Option<Self> {
        let ico = load_tray_icon(&params.icon_path)?;
        let builder = tray_icon::TrayIconBuilder::new();
        #[cfg(target_os = "macos")]
        let builder = if params.icon_is_template {
            builder.with_icon_templated(ico)
        } else {
            builder.with_icon(ico)
        };
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(ico);
        let mut builder = builder.with_menu_on_left_click(params.menu_on_left_click);
        // The tooltip is the icon's accessible name: fall back to the window title
        let tooltip = params.tooltip.clone().unwrap_or_else(|| window.title());
        if !tooltip.is_empty() {
            builder = builder.with_tooltip(tooltip);
        }
        if let Some(ref title) = params.title {
            builder = builder.with_title(title);
        }
        let mut has_menu = false;
        if let Some(ref menu_json) = params.menu_json {
            if let Some(built) = menu::build_menu(menu_json, &menu::about_metadata(config)) {
                has_menu = !built.menu.items().is_empty();
                menu_items.replace(menu::MenuOwner::Tray, built.items);
                builder = builder.with_menu(Box::new(built.menu));
            }
        }
        builder.build().ok().map(|icon| Tray {
            icon,
            has_menu,
            menu_on_left_click: params.menu_on_left_click,
        })
    }

    /// Whether clicking with this button shows the tray menu (macOS, Windows).
    pub(crate) fn opens_menu(&self, button: tray_icon::MouseButton) -> bool {
        self.has_menu
            && match button {
                tray_icon::MouseButton::Left => self.menu_on_left_click,
                tray_icon::MouseButton::Right => true,
                tray_icon::MouseButton::Middle => false,
            }
    }
}

fn load_tray_icon(path: &str) -> Option<tray_icon::Icon> {
    // Windows: an .ico is loaded by the system at the small icon size, from its closest frame. Decoded as an image
    // its largest frame would be used, and the shell shrinks that one without filtering (jagged edges).
    #[cfg(target_os = "windows")]
    if path.to_ascii_lowercase().ends_with(".ico") {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) } as u32;
        if let Ok(icon) = tray_icon::Icon::from_path(path, Some((size, size))) {
            return Some(icon);
        }
    }
    let img = image::open(path).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()
}
