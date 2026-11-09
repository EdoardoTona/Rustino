//! C API of the window: lifecycle, settings, state and the handlers of its events.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;
use std::sync::atomic::Ordering;

use crate::commands::RustinoCommand;
use crate::config::{AboutField, RustinoInitParams, WindowConfig};
use crate::util::{self, panic_message};

use super::{RustinoWindow, Setup};

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_ctor(params: *const RustinoInitParams) -> *mut RustinoWindow {
    catch_unwind(|| {
        if params.is_null() {
            return std::ptr::null_mut();
        }
        let config = WindowConfig::from_params(unsafe { &*params });
        let instance = RustinoWindow::new(config);
        Box::into_raw(Box::new(instance))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Frees the window. A running window (e.g. destroyed from one of its handlers) closes, and is
/// freed when `rustino_wait_for_exit` returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_dtor(instance: *mut RustinoWindow) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.close_for_destroy();
            if inst.release() {
                unsafe { drop(Box::from_raw(instance)) };
            }
        }
    }));
}

/// Runs the window until it closes. Returns 0 when it closed, 1 when it failed: the window or the
/// webview couldn't be created, it already ran, or the native code panicked.
/// The first window runs the event loop of the app on the calling thread (on macOS, the main
/// thread). The windows started while it runs join that event loop, from any other thread, and
/// close with the first one.
/// `error_out` receives an owned UTF-8 error string on failure; free it with
/// `rustino_free_string`. When the host destroyed the window meanwhile, it's freed before
/// returning.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_wait_for_exit(
    instance: *mut RustinoWindow,
    error_out: *mut *mut c_char,
) -> i32 {
    if let Some(error_out) = unsafe { error_out.as_mut() } {
        *error_out = std::ptr::null_mut();
    }
    let Some(inst) = (unsafe { instance.as_ref() }) else {
        if let Some(error_out) = unsafe { error_out.as_mut() } {
            *error_out = std::ffi::CString::new("The native window instance is null.")
                .expect("static string has no NUL")
                .into_raw();
        }
        return 1;
    };
    let mut ran = false;
    let result = catch_unwind(std::panic::AssertUnwindSafe(|| {
        let started = inst.start()?;
        ran = true;
        inst.run(started)
    }))
    .unwrap_or_else(|panic| Err(panic_message(panic.as_ref())));
    let status = match result {
        Ok(()) => 0,
        Err(message) => {
            if let Some(error_out) = unsafe { error_out.as_mut() } {
                *error_out = std::ffi::CString::new(message.replace('\0', " "))
                    .unwrap_or_default()
                    .into_raw();
            }
            1
        }
    };
    if ran && inst.release() {
        unsafe { drop(Box::from_raw(instance)) };
    }
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_close(instance: *mut RustinoWindow) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::Close);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_free_string(s: *mut c_char) {
    let _ = catch_unwind(|| {
        util::free_cstring(s);
    });
}

// ---------------------------------------------------------------------------
// Dual-mode setters (pre-run: modify config, then send a command, queued while the window starts)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_title(instance: *mut RustinoWindow, title: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(t) = unsafe { util::cstr_to_string(title) } {
                inst.set(RustinoCommand::SetTitle(t.clone()), |s| s.config.title = t);
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_size(instance: *mut RustinoWindow, width: i32, height: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let w = width.max(1) as u32;
            let h = height.max(1) as u32;
            inst.set(RustinoCommand::SetSize(w, h), |s| {
                s.config.width = w;
                s.config.height = h;
            });
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_resizable(instance: *mut RustinoWindow, resizable: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let v = resizable != 0;
            inst.set(RustinoCommand::SetResizable(v), |s| s.config.resizable = v);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_topmost(instance: *mut RustinoWindow, topmost: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let v = topmost != 0;
            inst.set(RustinoCommand::SetTopmost(v), |s| s.config.topmost = v);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_icon_file(instance: *mut RustinoWindow, path: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(p) = unsafe { util::cstr_to_string(path) } {
                inst.set(RustinoCommand::SetIconFile(p.clone()), |s| s.config.icon_file = Some(p));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_center(instance: *mut RustinoWindow) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.set(RustinoCommand::Center, |s| s.config.center = true);
        }
    });
}

fn set_about(instance: *mut RustinoWindow, field: AboutField, value: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let v = unsafe { util::cstr_to_string(value) };
            inst.set(RustinoCommand::SetAbout(field, v.clone()), |s| s.config.set_about(field, v));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_about_name(instance: *mut RustinoWindow, name: *const c_char) {
    set_about(instance, AboutField::Name, name);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_about_version(instance: *mut RustinoWindow, version: *const c_char) {
    set_about(instance, AboutField::Version, version);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_about_copyright(instance: *mut RustinoWindow, copyright: *const c_char) {
    set_about(instance, AboutField::Copyright, copyright);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_about_website(instance: *mut RustinoWindow, website: *const c_char) {
    set_about(instance, AboutField::Website, website);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_about_license(instance: *mut RustinoWindow, license: *const c_char) {
    set_about(instance, AboutField::License, license);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_about_authors(instance: *mut RustinoWindow, authors: *const c_char) {
    set_about(instance, AboutField::Authors, authors);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_about_comments(instance: *mut RustinoWindow, comments: *const c_char) {
    set_about(instance, AboutField::Comments, comments);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_navigate_to_url(instance: *mut RustinoWindow, url: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(u) = unsafe { util::cstr_to_string(url) } {
                inst.set(RustinoCommand::LoadUrl(u.clone()), |s| {
                    s.config.start_url = Some(u);
                    s.config.start_html = None;
                });
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_navigate_to_string(
    instance: *mut RustinoWindow,
    content: *const c_char,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() }
            && let Some(c) = unsafe { util::cstr_to_string(content) }
        {
            inst.set(RustinoCommand::LoadHtml(c.clone()), |s| {
                s.config.start_html = Some(c);
                s.config.start_url = None;
            });
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_background_color(
    instance: *mut RustinoWindow,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.set(RustinoCommand::SetBackgroundColor(r, g, b, a), |s| {
                s.config.background_color = Some((r, g, b, a))
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Pre-run only setters (builder-time configuration): 1 when applied, 0 once the window started
// ---------------------------------------------------------------------------

/// Changes the configuration before the window runs: 1 when applied, 0 once it started. Shared
/// by the C API of the features, which keep their settings in the configuration.
pub(crate) unsafe fn configure(instance: *mut RustinoWindow, store: impl FnOnce(&mut Setup)) -> i32 {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        unsafe { instance.as_ref() }.map_or(0, |inst| i32::from(inst.configure(store)))
    }))
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_use_os_default_size(instance: *mut RustinoWindow, use_default: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.use_os_default_size = use_default != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_devtools_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.devtools_enabled = enabled != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_clipboard_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.clipboard_enabled = enabled != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_ignore_cert_errors(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.ignore_certificate_errors = enabled != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_web_security_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.web_security_enabled = enabled != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_transparent(instance: *mut RustinoWindow, transparent: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.transparent = transparent != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_decorations(instance: *mut RustinoWindow, decorated: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let v = decorated != 0;
            inst.set(RustinoCommand::SetDecorations(v), |s| s.config.decorations = v);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_user_agent(instance: *mut RustinoWindow, ua: *const c_char) -> i32 {
    let ua = unsafe { util::cstr_to_string(ua) };
    unsafe { configure(instance, |s| s.config.user_agent = ua) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_user_data_folder(instance: *mut RustinoWindow, path: *const c_char) -> i32 {
    let path = unsafe { util::cstr_to_string(path) };
    unsafe { configure(instance, |s| s.config.user_data_folder = path) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_media_autoplay(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.media_autoplay = enabled != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_zoom_hotkeys(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { configure(instance, |s| s.config.zoom_hotkeys = enabled != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_add_init_script(instance: *mut RustinoWindow, js: *const c_char) -> i32 {
    let script = unsafe { util::cstr_to_string(js) };
    unsafe { configure(instance, |s| s.config.initialization_scripts.extend(script)) }
}

// ---------------------------------------------------------------------------
// Window state (commands, queued until the window runs)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_minimized(instance: *mut RustinoWindow, minimized: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::SetMinimized(minimized != 0));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_maximized(instance: *mut RustinoWindow, maximized: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let v = maximized != 0;
            inst.set(RustinoCommand::SetMaximized(v), |s| s.config.maximized = v);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_fullscreen(instance: *mut RustinoWindow, fullscreen: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let v = fullscreen != 0;
            inst.set(RustinoCommand::SetFullscreen(v), |s| s.config.fullscreen = v);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_visible(instance: *mut RustinoWindow, visible: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let v = visible != 0;
            inst.set(RustinoCommand::SetVisible(v), |s| s.config.visible = v);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_focus(instance: *mut RustinoWindow) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::SetFocus);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_position(instance: *mut RustinoWindow, x: i32, y: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.set(RustinoCommand::SetPosition(x, y), |s| s.config.position = Some((x, y)));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_min_size(instance: *mut RustinoWindow, width: i32, height: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let size = if width > 0 && height > 0 {
                Some((width as u32, height as u32))
            } else {
                None
            };
            inst.set(RustinoCommand::SetMinSize(size), |s| s.config.min_size = size);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_max_size(instance: *mut RustinoWindow, width: i32, height: i32) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let size = if width > 0 && height > 0 {
                Some((width as u32, height as u32))
            } else {
                None
            };
            inst.set(RustinoCommand::SetMaxSize(size), |s| s.config.max_size = size);
        }
    });
}

// ---------------------------------------------------------------------------
// State queries (read from shared state, no round-trip)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_is_minimized(instance: *mut RustinoWindow) -> i32 {
    catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if inst.state.is_minimized.load(Ordering::Acquire) { 1 } else { 0 }
        } else {
            0
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_is_maximized(instance: *mut RustinoWindow) -> i32 {
    catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if inst.state.is_maximized.load(Ordering::Acquire) { 1 } else { 0 }
        } else {
            0
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_is_fullscreen(instance: *mut RustinoWindow) -> i32 {
    catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if inst.state.is_fullscreen.load(Ordering::Acquire) { 1 } else { 0 }
        } else {
            0
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_position(
    instance: *mut RustinoWindow,
    out_x: *mut i32,
    out_y: *mut i32,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let (x, y) = inst.state.load_position();
            if !out_x.is_null() {
                unsafe { *out_x = x };
            }
            if !out_y.is_null() {
                unsafe { *out_y = y };
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_size(
    instance: *mut RustinoWindow,
    out_w: *mut i32,
    out_h: *mut i32,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let (w, h) = inst.state.load_size();
            if !out_w.is_null() {
                unsafe { *out_w = w as i32 };
            }
            if !out_h.is_null() {
                unsafe { *out_h = h as i32 };
            }
        }
    });
}

// ---------------------------------------------------------------------------
// WebView operations (queued until the window runs)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_evaluate_script(instance: *mut RustinoWindow, js: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(s) = unsafe { util::cstr_to_string(js) } {
                inst.send_command(RustinoCommand::EvaluateScript(s));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_send_web_message(instance: *mut RustinoWindow, msg: *const c_char) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(m) = unsafe { util::cstr_to_string(msg) } {
                inst.send_command(RustinoCommand::SendWebMessage(m));
            }
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_zoom(instance: *mut RustinoWindow, factor: f64) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if factor.is_finite() && factor > 0.0 {
                inst.send_command(RustinoCommand::SetZoom(factor));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Callback registration (pre-run only, ignored once the window started)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_callback_context(
    instance: *mut RustinoWindow,
    context: *mut c_void,
) {
    unsafe { configure(instance, |s| s.callbacks.context = context) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_closing_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_closing = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_closed_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_closed = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_resized_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32, i32)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_resized = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_moved_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32, i32)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_moved = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_focus_changed_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_focus_changed = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_web_message_received_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_web_message = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_page_load_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32, *const c_char)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_page_load = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_navigation_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> i32>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_navigation = handler) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_runs_once() {
        let instance = Box::into_raw(Box::new(RustinoWindow::new(WindowConfig::default())));
        unsafe {
            assert!(instance.as_ref().unwrap().start().is_ok());
            // The window is starting: a second run fails without touching it
            let mut error = std::ptr::null_mut();
            assert_eq!(rustino_wait_for_exit(instance, &mut error), 1);
            assert_eq!(std::ffi::CStr::from_ptr(error).to_str().unwrap(), "The window is already running.");
            rustino_free_string(error);
            // Destroyed while it runs: freed by the first run, once done
            rustino_dtor(instance);
            assert!(instance.as_ref().unwrap().release());
            drop(Box::from_raw(instance));
        }
    }
}
