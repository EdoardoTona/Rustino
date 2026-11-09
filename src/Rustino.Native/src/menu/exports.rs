//! C API of the menus: the menu bar, context menus and their items. Queued until the window runs.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;

use crate::commands::RustinoCommand;
use crate::util;
use crate::window::{RustinoWindow, configure};

use super::MenuItemUpdate;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu(instance: *mut RustinoWindow, json: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(j) = unsafe { util::cstr_to_string(json) } {
                inst.send_command(RustinoCommand::SetMenu(j));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_remove_menu(instance: *mut RustinoWindow) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::RemoveMenu);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_show_context_menu(
    instance: *mut RustinoWindow,
    json: *const c_char,
    x: f64,
    y: f64,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(j) = unsafe { util::cstr_to_string(json) } {
                let pos = if x < 0.0 && y < 0.0 {
                    None
                } else {
                    Some((x, y))
                };
                inst.send_command(RustinoCommand::ShowContextMenu(j, pos));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu_item_enabled(
    instance: *mut RustinoWindow,
    id: *const c_char,
    enabled: i32,
) {
    unsafe { update_menu_item(instance, id, MenuItemUpdate::Enabled(enabled != 0)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu_item_checked(
    instance: *mut RustinoWindow,
    id: *const c_char,
    checked: i32,
) {
    unsafe { update_menu_item(instance, id, MenuItemUpdate::Checked(checked != 0)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu_item_text(
    instance: *mut RustinoWindow,
    id: *const c_char,
    text: *const c_char,
) {
    if let Some(text) = unsafe { util::cstr_to_string(text) } {
        unsafe { update_menu_item(instance, id, MenuItemUpdate::Text(text)) };
    }
}

unsafe fn update_menu_item(
    instance: *mut RustinoWindow,
    id: *const c_char,
    update: MenuItemUpdate,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(id) = unsafe { util::cstr_to_string(id) } {
                inst.send_command(RustinoCommand::UpdateMenuItem(id, update));
            }
        }
    });
}

/// Pre-run only, ignored once the window started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu_event_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char, i32)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_menu_item_clicked = handler) };
}
