//! The monitors of the system and the cursor position on them. tao knows them on the event
//! loop thread only: the monitors are cached in the state of the window, as JSON for the host.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;

use crate::state::SharedState;
use crate::window::{RustinoWindow, configure};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_monitors(instance: *mut RustinoWindow) -> *mut c_char {
    catch_unwind(|| {
        let inst = unsafe { instance.as_ref() }?;
        let json = inst.state.load_monitors();
        if json.is_empty() { return None; }
        std::ffi::CString::new(json).ok().map(|s| s.into_raw())
    })
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_current_monitor(instance: *mut RustinoWindow) -> *mut c_char {
    catch_unwind(|| {
        let inst = unsafe { instance.as_ref() }?;
        let json = inst.state.load_current_monitor();
        if json.is_empty() { return None; }
        std::ffi::CString::new(json).ok().map(|s| s.into_raw())
    })
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

/// Has the event loop give the monitors to the monitors handler, while the window runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_check_monitors(instance: *mut RustinoWindow) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.check_monitors();
        }
    }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_monitors_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_monitors = handler) };
}

/// The cursor position on the screen, in physical pixels. 1 when it's known, 0 when the window
/// doesn't run (on Wayland it's always 0, 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_cursor_position(instance: *mut RustinoWindow, x: *mut i32, y: *mut i32) -> i32 {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(inst) = (unsafe { instance.as_ref() }) else {
            return 0;
        };
        let Some(Ok(position)) = inst.invoke(|window, _| window.cursor_position()) else {
            return 0;
        };
        unsafe {
            if let Some(x) = x.as_mut() {
                *x = position.x.round() as i32;
            }
            if let Some(y) = y.as_mut() {
                *y = position.y.round() as i32;
            }
        }
        1
    }))
    .unwrap_or(0)
}

pub(crate) fn update_monitor_cache(window: &tao::window::Window, state: &SharedState) {
    let primary = window.primary_monitor();
    let monitors: Vec<serde_json::Value> = window
        .available_monitors()
        .map(|m| {
            let pos = m.position();
            let size = m.size();
            let is_primary = primary
                .as_ref()
                .map_or(false, |p| p.name() == m.name() && p.position() == m.position());
            serde_json::json!({
                "name": m.name(),
                "x": pos.x,
                "y": pos.y,
                "width": size.width,
                "height": size.height,
                "scaleFactor": m.scale_factor(),
                "isPrimary": is_primary
            })
        })
        .collect();
    let monitors_json = serde_json::to_string(&monitors).unwrap_or_default();

    let current_json = if let Some(m) = window.current_monitor() {
        let pos = m.position();
        let size = m.size();
        let is_primary = primary
            .as_ref()
            .map_or(false, |p| p.name() == m.name() && p.position() == m.position());
        serde_json::to_string(&serde_json::json!({
            "name": m.name(),
            "x": pos.x,
            "y": pos.y,
            "width": size.width,
            "height": size.height,
            "scaleFactor": m.scale_factor(),
            "isPrimary": is_primary
        }))
        .unwrap_or_default()
    } else {
        String::new()
    };

    state.store_monitors(&monitors_json, &current_json);
}
