//! C API of the webview features.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;

use crate::commands::RustinoCommand;
use crate::invoke;
use crate::util;
use crate::window::{RustinoWindow, configure};

/// Changes a webview option before the window runs: 1 when applied, 0 once it started.
unsafe fn set_webview_option(
    instance: *mut RustinoWindow,
    store: impl FnOnce(&mut super::WebViewExtOptions),
) -> i32 {
    unsafe { configure(instance, |s| store(&mut s.webview_ext.options)) }
}

/// Runs a webview operation on the event loop thread, without waiting for it (once the window runs).
unsafe fn post_webview(instance: *mut RustinoWindow, operation: impl FnOnce(&wry::WebView) + Send + 'static) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.post(move |_, webview| operation(webview));
        }
    }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_context_menu_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { set_webview_option(instance, |o| o.context_menu = enabled != 0) }
}

/// Windows only
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_browser_accelerator_keys_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { set_webview_option(instance, |o| o.browser_accelerator_keys = enabled != 0) }
}

/// Windows only: 0 default, 1 Fluent overlay
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_scroll_bar_style(instance: *mut RustinoWindow, style: i32) -> i32 {
    unsafe { set_webview_option(instance, |o| o.fluent_overlay_scroll_bars = style == 1) }
}

/// macOS only
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_accept_first_mouse(instance: *mut RustinoWindow, accept: i32) -> i32 {
    unsafe { set_webview_option(instance, |o| o.accept_first_mouse = accept != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_back_forward_gestures_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { set_webview_option(instance, |o| o.back_forward_gestures = enabled != 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_file_drop_enabled(instance: *mut RustinoWindow, enabled: i32) -> i32 {
    unsafe { set_webview_option(instance, |o| o.file_drop = enabled != 0) }
}

/// The system print dialog
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_print(instance: *mut RustinoWindow) {
    unsafe { post_webview(instance, super::print) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_reload(instance: *mut RustinoWindow) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::Reload);
        }
    });
}

/// Needs `rustino_set_devtools_enabled`; on macOS also the `devtools` feature
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_open_devtools(instance: *mut RustinoWindow) {
    unsafe { post_webview(instance, super::open_devtools) };
}

/// Not supported on Windows
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_close_devtools(instance: *mut RustinoWindow) {
    unsafe { post_webview(instance, super::close_devtools) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_clear_browsing_data(instance: *mut RustinoWindow) {
    unsafe { post_webview(instance, |webview| { let _ = webview.clear_all_browsing_data(); }) };
}

/// JSON array of the cookies (all of them with a null `url`). `status`: 0 done, 1 the window
/// doesn't run or the webview failed, 2 called within a webview event on Windows, where WebView2
/// answers only after the event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_get_cookies(
    instance: *mut RustinoWindow,
    url: *const c_char,
    status: *mut i32,
) -> *mut c_char {
    let (json, code) = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !invoke::can_wait_for_webview() {
            return (None, 2);
        }
        let json = unsafe { instance.as_ref() }.and_then(|inst| {
            let url = unsafe { util::cstr_to_string(url) };
            inst.invoke(move |_, webview| super::cookies_json(webview, url.as_deref()))?
        });
        let code = if json.is_some() { 0 } else { 1 };
        (json.and_then(|j| std::ffi::CString::new(j).ok()), code)
    }))
    .unwrap_or((None, 1));
    if let Some(status) = unsafe { status.as_mut() } {
        *status = code;
    }
    json.map_or(std::ptr::null_mut(), |s| s.into_raw())
}

/// Adds or replaces a cookie (JSON object). Returns 1 on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_cookie(instance: *mut RustinoWindow, json: *const c_char) -> i32 {
    unsafe { update_cookie(instance, json, |webview, cookie| webview.set_cookie(cookie).is_ok()) }
}

/// Deletes a cookie (JSON object: name, domain and path identify it). Returns 1 on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_delete_cookie(instance: *mut RustinoWindow, json: *const c_char) -> i32 {
    unsafe { update_cookie(instance, json, |webview, cookie| webview.delete_cookie(cookie).is_ok()) }
}

unsafe fn update_cookie(
    instance: *mut RustinoWindow,
    json: *const c_char,
    update: impl FnOnce(&wry::WebView, &wry::cookie::Cookie<'static>) -> bool + Send + 'static,
) -> i32 {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let inst = unsafe { instance.as_ref() }?;
        let json = unsafe { util::cstr_to_string(json) }?;
        let cookie = serde_json::from_str::<super::CookieData>(&json).ok()?.to_cookie();
        inst.invoke(move |_, webview| update(webview, &cookie))
    }))
    .ok()
    .flatten()
    .map_or(0, i32::from)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_file_drop_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32, *const c_char, i32, i32)>,
) {
    unsafe { configure(instance, |s| s.webview_ext.callbacks.on_file_drop = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_document_title_changed_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
) {
    unsafe { configure(instance, |s| s.webview_ext.callbacks.on_document_title_changed = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_download_starting_handler(
    instance: *mut RustinoWindow,
    handler: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut super::DownloadResponse) -> i32,
    >,
) {
    unsafe { configure(instance, |s| s.webview_ext.callbacks.on_download_starting = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_download_completed_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, i32)>,
) {
    unsafe { configure(instance, |s| s.webview_ext.callbacks.on_download_completed = handler) };
}

/// Called by the host from within the download starting callback: where to save the file
/// instead of asking the user.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_download_destination(
    response: *mut super::DownloadResponse,
    path: *const c_char,
) {
    let _ = catch_unwind(|| {
        if let Some(r) = unsafe { response.as_mut() } {
            r.destination = unsafe { util::cstr_to_string(path) };
        }
    });
}
