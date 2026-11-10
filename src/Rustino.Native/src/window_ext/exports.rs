//! C API of the window features.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;

use crate::commands::RustinoCommand;
use crate::util;
use crate::window::{RustinoWindow, configure};

/// Sends a window feature command to the running window; before it runs, `store` keeps the
/// setting in the options.
unsafe fn set_window_feature(
    instance: *mut RustinoWindow,
    command: super::WindowCommand,
    store: impl FnOnce(&mut super::WindowExtOptions),
) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.set(RustinoCommand::Window(command), |s| store(&mut s.ext.options));
        }
    }));
}

/// Sends a window feature command, queued until the window runs.
unsafe fn run_window_feature(instance: *mut RustinoWindow, command: super::WindowCommand) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::Window(command));
        }
    }));
}

/// 0 follows the system, 1 light, 2 dark
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_theme(instance: *mut RustinoWindow, theme: i32) {
    let theme = super::theme_from_i32(theme);
    unsafe { set_window_feature(instance, super::WindowCommand::SetTheme(theme), |o| o.theme = theme) };
}

/// 1 light, 2 dark, 0 before the window runs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_theme(instance: *mut RustinoWindow) -> i32 {
    catch_unwind(|| unsafe { instance.as_ref() }.map_or(0, |inst| i32::from(inst.state.load_theme())))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_scale_factor(instance: *mut RustinoWindow) -> f64 {
    catch_unwind(|| unsafe { instance.as_ref() }.map_or(1.0, |inst| inst.state.load_scale_factor()))
        .unwrap_or(1.0)
}

/// `state`: 0 none, 1 normal, 2 indeterminate, 3 paused, 4 error. `progress`: 0-100, negative
/// keeps the current value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_progress_bar(instance: *mut RustinoWindow, state: i32, progress: i32) {
    let state = super::progress_state_from_i32(state);
    let progress = u64::try_from(progress).ok();
    unsafe { run_window_feature(instance, super::WindowCommand::SetProgressBar(state, progress)) };
}

/// 0 cancels the request, 1 informational, 2 critical
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_request_user_attention(instance: *mut RustinoWindow, kind: i32) {
    let kind = super::attention_from_i32(kind);
    unsafe { run_window_feature(instance, super::WindowCommand::RequestUserAttention(kind)) };
}

/// Plays the system alert sound
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_beep(instance: *mut RustinoWindow) {
    unsafe { run_window_feature(instance, super::WindowCommand::Beep) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_shadow(instance: *mut RustinoWindow, shadow: i32) {
    let v = shadow != 0;
    unsafe { set_window_feature(instance, super::WindowCommand::SetShadow(v), |o| o.shadow = Some(v)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_skip_taskbar(instance: *mut RustinoWindow, skip: i32) {
    let v = skip != 0;
    unsafe { set_window_feature(instance, super::WindowCommand::SetSkipTaskbar(v), |o| o.skip_taskbar = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_content_protection(instance: *mut RustinoWindow, enabled: i32) {
    let v = enabled != 0;
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetContentProtection(v), |o| {
            o.content_protection = v
        })
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_visible_on_all_workspaces(instance: *mut RustinoWindow, visible: i32) {
    let v = visible != 0;
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetVisibleOnAllWorkspaces(v), |o| {
            o.visible_on_all_workspaces = v
        })
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_closable(instance: *mut RustinoWindow, closable: i32) {
    let v = closable != 0;
    unsafe { set_window_feature(instance, super::WindowCommand::SetClosable(v), |o| o.closable = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_minimizable(instance: *mut RustinoWindow, minimizable: i32) {
    let v = minimizable != 0;
    unsafe { set_window_feature(instance, super::WindowCommand::SetMinimizable(v), |o| o.minimizable = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_maximizable(instance: *mut RustinoWindow, maximizable: i32) {
    let v = maximizable != 0;
    unsafe { set_window_feature(instance, super::WindowCommand::SetMaximizable(v), |o| o.maximizable = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_always_on_bottom(instance: *mut RustinoWindow, on_bottom: i32) {
    let v = on_bottom != 0;
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetAlwaysOnBottom(v), |o| o.always_on_bottom = v)
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_ignore_cursor_events(instance: *mut RustinoWindow, ignore: i32) {
    let v = ignore != 0;
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetIgnoreCursorEvents(v), |o| {
            o.ignore_cursor_events = v
        })
    };
}

/// 0 default, 1 transparent, 2 overlay (see `MacTitleBarStyle`)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_mac_title_bar_style(instance: *mut RustinoWindow, style: i32) {
    let style = super::MacTitleBarStyle::from_i32(style);
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetMacTitleBarStyle(style), |o| {
            o.mac_title_bar_style = style
        })
    };
}

/// Windows and Linux: the page extends to the top of the window, under window controls drawn by
/// the drag region script. Ignored on macOS (see `rustino_set_mac_title_bar_style`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_title_bar_overlay(instance: *mut RustinoWindow, overlay: i32) {
    let v = overlay != 0;
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetTitleBarOverlay(v), |o| {
            o.title_bar_overlay = v
        })
    };
}

/// Color of the overlay title bar buttons; `a` 0 goes back to the page's color
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_title_bar_overlay_color(
    instance: *mut RustinoWindow,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    let color = (a != 0).then_some((r, g, b, a));
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetTitleBarOverlayColor(color), |o| {
            o.title_bar_overlay_color = color
        })
    };
}

/// Height of the overlay title bar in logical pixels; 0 or less goes back to the platform's
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_title_bar_overlay_height(instance: *mut RustinoWindow, height: i32) {
    let height = u32::try_from(height).ok().filter(|&h| h > 0);
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetTitleBarOverlayHeight(height), |o| {
            o.title_bar_overlay_height = height
        })
    };
}

/// Kiosk mode: the window covers its screen and stays in front, and the user can't close it,
/// minimize it or switch to another app, until the host turns it off (see `kiosk`).
/// `without_touchpad_gestures` (Windows): the gestures with three and four fingers of the user's
/// touchpad settings are off meanwhile.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_kiosk(instance: *mut RustinoWindow, kiosk: i32, without_touchpad_gestures: i32) {
    let (on, gestures_off) = (kiosk != 0, without_touchpad_gestures != 0);
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetKiosk(on, gestures_off), |o| {
            o.kiosk = on;
            o.kiosk_without_touchpad_gestures = gestures_off;
        })
    };
}

/// Whether the running window is in kiosk mode
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_is_kiosk(instance: *mut RustinoWindow) -> i32 {
    use std::sync::atomic::Ordering;
    catch_unwind(|| unsafe { instance.as_ref() }.map_or(0, |inst| i32::from(inst.state.is_kiosk.load(Ordering::Acquire))))
        .unwrap_or(0)
}

/// Pre-run only: 0 once the window started. Without drag regions no page can move, resize or
/// maximize the window through the `__rustino:` messages, which reach the host instead.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_drag_regions_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { configure(instance, |s| s.ext.options.drag_regions = enabled != 0) }
}

/// Position of the macOS traffic lights, in logical pixels from the top-left corner
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_traffic_light_position(instance: *mut RustinoWindow, x: f64, y: f64) {
    unsafe {
        set_window_feature(instance, super::WindowCommand::SetTrafficLightPosition(x, y), |o| {
            o.traffic_light_position = Some((x, y))
        })
    };
}

/// Linux: the `.desktop` file of the app, whose dock icon shows the badge and the progress
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_desktop_file_name(instance: *mut RustinoWindow, name: *const c_char) {
    let name = unsafe { util::cstr_to_string(name) };
    let command = super::WindowCommand::SetDesktopFileName(name.clone());
    unsafe { set_window_feature(instance, command, |o| o.desktop_file_name = name) };
}

/// Moves the window with the mouse: call it while the left button is down.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_drag_window(instance: *mut RustinoWindow) {
    unsafe { run_window_feature(instance, super::WindowCommand::DragWindow) };
}

/// Resizes the window with the mouse from an edge: 0 north, then clockwise to 7 north-west
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_drag_resize_window(instance: *mut RustinoWindow, direction: i32) {
    if let Some(direction) = super::direction_from_i32(direction) {
        unsafe { run_window_feature(instance, super::WindowCommand::DragResizeWindow(direction)) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_theme_changed_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32)>,
) {
    unsafe { configure(instance, |s| s.ext.callbacks.on_theme_changed = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_scale_factor_changed_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, f64, i32, i32)>,
) {
    unsafe { configure(instance, |s| s.ext.callbacks.on_scale_factor_changed = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_urls_opened_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
) {
    unsafe { configure(instance, |s| s.ext.callbacks.on_urls_opened = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_reopen_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32)>,
) {
    unsafe { configure(instance, |s| s.ext.callbacks.on_reopen = handler) };
}
