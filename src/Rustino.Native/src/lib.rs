#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::unnecessary_map_or)]

#[cfg(test)]
mod abi_layout;
#[cfg(target_os = "windows")]
mod accelerators;
mod callbacks;
mod commands;
mod config;
mod dialogs;
mod icon;
mod invoke;
mod menu;
mod splash;
mod state;
mod util;
mod webview_ext;
mod window;
mod window_ext;

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::catch_unwind;
use std::sync::atomic::Ordering;

use commands::RustinoCommand;
use config::{AboutField, RustinoInitParams, WindowConfig};
use window::RustinoWindow;

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

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown error".into());
    format!("The native window failed: {detail}")
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
// Notifications (standalone — no instance required)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_show_notification(
    title: *const c_char,
    body: *const c_char,
    icon: *const c_char,
    app_id: *const c_char,
) -> i32 {
    catch_unwind(|| {
        let title = unsafe { util::cstr_to_string(title) }.unwrap_or_default();
        let body = unsafe { util::cstr_to_string(body) }.unwrap_or_default();
        let mut n = notify_rust::Notification::new();
        n.summary(&title).body(&body);
        if let Some(icon_path) = unsafe { util::cstr_to_string(icon) } {
            n.icon(&icon_path);
        }
        #[cfg(target_os = "windows")]
        if let Some(id) = unsafe { util::cstr_to_string(app_id) } {
            n.app_id(&id);
        }
        #[cfg(target_os = "macos")]
        {
            let requested = unsafe { util::cstr_to_string(app_id) };
            macos_notification::ensure_application_set(requested.as_deref());
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let _ = app_id;
        if n.show().is_ok() { 1 } else { 0 }
    })
    .unwrap_or(0)
}

// notify-rust's macOS backend lazily calls `set_application("use_default")` on the
// first notification if nobody has set one, which makes macOS pop a "Where is
// use_default?" app-picker dialog (see Ivy-Tendril#1682). We must call
// `notify_rust::set_application` ourselves before that happens — and since its
// internal `Once` latches even on failure, it can only ever be called once, with
// an id we already know is valid.
#[cfg(target_os = "macos")]
mod macos_notification {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSBundle, NSString};
    use std::sync::Once;

    static ENSURE_APPLICATION_SET: Once = Once::new();

    /// Picks a LaunchServices-registered bundle id and calls `notify_rust::set_application`
    /// with it exactly once. Must be called before the first `Notification::show()`.
    pub fn ensure_application_set(requested_app_id: Option<&str>) {
        ENSURE_APPLICATION_SET.call_once(|| {
            let bundle_id = pick_bundle_id(requested_app_id);
            let _ = notify_rust::set_application(&bundle_id);
        });
    }

    /// Returns the first candidate that is actually registered with LaunchServices,
    /// falling back to "com.apple.Finder" (guaranteed installed; notify-rust's own
    /// eventual default) if nothing else resolves.
    fn pick_bundle_id(requested_app_id: Option<&str>) -> String {
        let main_bundle_id = main_bundle_identifier();
        let candidates = [
            requested_app_id,
            main_bundle_id.as_deref(),
            Some("com.apple.Terminal"),
        ];
        candidates
            .into_iter()
            .flatten()
            .find(|id| !id.is_empty() && is_registered_bundle_id(id))
            .map(str::to_string)
            .unwrap_or_else(|| "com.apple.Finder".to_string())
    }

    /// The running app's own bundle identifier, when running inside a real .app bundle.
    fn main_bundle_identifier() -> Option<String> {
        NSBundle::mainBundle().bundleIdentifier().map(|s| s.to_string())
    }

    /// LaunchServices only accepts bundle ids it knows about; `setApplication` silently
    /// rejects anything else, so we pre-validate via the same lookup it uses internally
    /// (`LSCopyApplicationURLsForBundleIdentifier`, exposed here as `URLForApplication...`).
    fn is_registered_bundle_id(bundle_id: &str) -> bool {
        let workspace = NSWorkspace::sharedWorkspace();
        let id = NSString::from_str(bundle_id);
        workspace.URLForApplicationWithBundleIdentifier(&id).is_some()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_register_notification_app_id(
    app_id: *const c_char,
    display_name: *const c_char,
    icon: *const c_char,
) -> i32 {
    catch_unwind(|| {
        let Some(app_id) =
            unsafe { util::cstr_to_string(app_id) }.filter(|id| is_valid_app_user_model_id(id))
        else {
            return 0;
        };
        #[cfg(target_os = "windows")]
        {
            let display_name = unsafe { util::cstr_to_string(display_name) }
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| app_id.clone());
            let icon_path = unsafe { util::cstr_to_string(icon) }
                .filter(|path| !path.is_empty())
                .map(|path| {
                    std::path::absolute(&path)
                        .map(|abs| abs.to_string_lossy().into_owned())
                        .unwrap_or(path)
                });
            let registered =
                windows_notification::register_app_id(&app_id, &display_name, icon_path.as_deref());
            if registered { 1 } else { 0 }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (app_id, display_name, icon);
            1
        }
    })
    .unwrap_or(0)
}

/// Application-defined AppUserModelIDs have at most 128 characters and no spaces
/// (https://learn.microsoft.com/windows/win32/shell/appids). The id also becomes a
/// registry key name on Windows, so a backslash would nest keys.
fn is_valid_app_user_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.encode_utf16().count() <= 128
        && !id.chars().any(|c| c.is_whitespace() || c == '\\')
}

// Windows silently drops toasts whose AppUserModelID it cannot resolve: it needs a
// packaged app, a Start Menu shortcut carrying the id, or a registry registration.
// Unpackaged apps have none of these, so we write the per-user registration that
// Microsoft's own toolkits use for them (no shortcut, no admin rights needed).
#[cfg(target_os = "windows")]
mod windows_notification {
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
    };
    use windows::core::{HSTRING, PCWSTR};

    /// Creates (or updates) `HKCU\Software\Classes\AppUserModelId\<app_id>` with the name
    /// and icon Windows shows in the toast header. Without an icon, a previously
    /// registered one is removed so a stale path does not stay attached to the app.
    pub fn register_app_id(app_id: &str, display_name: &str, icon_path: Option<&str>) -> bool {
        let subkey = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{app_id}"));
        let mut key = HKEY::default();
        let created = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &subkey,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        };
        if created.is_err() {
            return false;
        }
        let ok = set_string(key, "DisplayName", display_name)
            && match icon_path {
                Some(icon) => set_string(key, "IconUri", icon),
                None => delete_value(key, "IconUri"),
            };
        unsafe {
            let _ = RegCloseKey(key);
        }
        ok
    }

    fn set_string(key: HKEY, name: &str, value: &str) -> bool {
        let data: Vec<u8> = value
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        unsafe { RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(&data)).is_ok() }
    }

    fn delete_value(key: HKEY, name: &str) -> bool {
        let result = unsafe { RegDeleteValueW(key, &HSTRING::from(name)) };
        result.is_ok() || result == ERROR_FILE_NOT_FOUND
    }
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

/// Changes the configuration before the window runs: 1 when applied, 0 once it started.
unsafe fn configure(instance: *mut RustinoWindow, store: impl FnOnce(&mut window::Setup)) -> i32 {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_add_custom_scheme(instance: *mut RustinoWindow, scheme: *const c_char) -> i32 {
    let scheme = unsafe { util::cstr_to_string(scheme) };
    unsafe { configure(instance, |s| s.config.custom_schemes.extend(scheme)) }
}

// ---------------------------------------------------------------------------
// Custom scheme response (called by the host from within the custom scheme callback)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_scheme_response(
    response: *mut window::SchemeResponse,
    data: *const u8,
    length: i32,
    content_type: *const c_char,
) {
    let _ = catch_unwind(|| {
        if let Some(r) = unsafe { response.as_mut() } {
            r.body = Some(if data.is_null() || length <= 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(data, length as usize) }.to_vec()
            });
            r.content_type = unsafe { util::cstr_to_string(content_type) };
        }
    });
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
// Dialogs (modal for the window while it runs, see dialogs)
// ---------------------------------------------------------------------------

unsafe fn file_dialog(
    instance: *mut RustinoWindow,
    kind: dialogs::FileDialogKind,
    title: *const c_char,
    default_path: *const c_char,
    filters: *const c_char,
) -> *mut c_char {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let params = dialogs::DialogParams {
            title: unsafe { util::cstr_to_string(title) },
            default_path: unsafe { util::cstr_to_string(default_path) },
            filters: dialogs::parse_filters(unsafe { util::cstr_to_string(filters) }),
        };
        let paths = dialogs::file_dialog(unsafe { instance.as_ref() }, kind, params)?;
        std::ffi::CString::new(paths.join("\n")).ok().map(|s| s.into_raw())
    }))
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

/// Paths separated by '\n', null when canceled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_show_open_file_dialog(
    instance: *mut RustinoWindow,
    title: *const c_char,
    default_path: *const c_char,
    filters: *const c_char,
    multi_select: i32,
) -> *mut c_char {
    let kind = dialogs::FileDialogKind::Open { multiple: multi_select != 0 };
    unsafe { file_dialog(instance, kind, title, default_path, filters) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_show_save_file_dialog(
    instance: *mut RustinoWindow,
    title: *const c_char,
    default_path: *const c_char,
    filters: *const c_char,
) -> *mut c_char {
    unsafe { file_dialog(instance, dialogs::FileDialogKind::Save, title, default_path, filters) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_show_select_folder_dialog(
    instance: *mut RustinoWindow,
    title: *const c_char,
    default_path: *const c_char,
    multi_select: i32,
) -> *mut c_char {
    let kind = dialogs::FileDialogKind::Folder { multiple: multi_select != 0 };
    unsafe { file_dialog(instance, kind, title, default_path, std::ptr::null()) }
}

/// `buttons`: 0 Ok, 1 OkCancel, 2 YesNo, 3 YesNoCancel, 4 RetryCancel, 5 AbortRetryIgnore.
/// `icon`: 0 info, 1 warning, 2 error, 3 question. Returns -1 cancel, 0 ok, 1 yes, 2 no, 3 abort,
/// 4 retry, 5 ignore. `instance` can be null: the dialog then has no parent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_show_message(
    instance: *mut RustinoWindow,
    title: *const c_char,
    text: *const c_char,
    buttons: i32,
    icon: i32,
) -> i32 {
    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let result = dialogs::message(
            unsafe { instance.as_ref() },
            unsafe { util::cstr_to_string(title) }.unwrap_or_default(),
            unsafe { util::cstr_to_string(text) }.unwrap_or_default(),
            dialogs::MessageButtons::from_i32(buttons),
            dialogs::MessageIcon::from_i32(icon),
        );
        result as i32
    }))
    .unwrap_or(dialogs::MessageResult::Cancel as i32)
}

// ---------------------------------------------------------------------------
// Taskbar badge (queued until the window runs)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_badge_count(
    instance: *mut RustinoWindow,
    count: i32,
    bg_r: u8,
    bg_g: u8,
    bg_b: u8,
    fg_r: u8,
    fg_g: u8,
    fg_b: u8,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            let badge = if count > 0 { Some(count as u32) } else { None };
            inst.send_command(RustinoCommand::SetBadgeCount {
                count: badge,
                bg_r,
                bg_g,
                bg_b,
                fg_r,
                fg_g,
                fg_b,
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Monitor enumeration (post-run only)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Window features (see window_ext)
// ---------------------------------------------------------------------------

/// Sends a window feature command to the running window; before it runs, `store` keeps the
/// setting in the options.
unsafe fn set_window_feature(
    instance: *mut RustinoWindow,
    command: window_ext::WindowCommand,
    store: impl FnOnce(&mut window_ext::WindowExtOptions),
) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.set(RustinoCommand::Window(command), |s| store(&mut s.ext.options));
        }
    }));
}

/// Sends a window feature command, queued until the window runs.
unsafe fn run_window_feature(instance: *mut RustinoWindow, command: window_ext::WindowCommand) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            inst.send_command(RustinoCommand::Window(command));
        }
    }));
}

/// 0 follows the system, 1 light, 2 dark
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_theme(instance: *mut RustinoWindow, theme: i32) {
    let theme = window_ext::theme_from_i32(theme);
    unsafe { set_window_feature(instance, window_ext::WindowCommand::SetTheme(theme), |o| o.theme = theme) };
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
    let state = window_ext::progress_state_from_i32(state);
    let progress = u64::try_from(progress).ok();
    unsafe { run_window_feature(instance, window_ext::WindowCommand::SetProgressBar(state, progress)) };
}

/// 0 cancels the request, 1 informational, 2 critical
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_request_user_attention(instance: *mut RustinoWindow, kind: i32) {
    let kind = window_ext::attention_from_i32(kind);
    unsafe { run_window_feature(instance, window_ext::WindowCommand::RequestUserAttention(kind)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_shadow(instance: *mut RustinoWindow, shadow: i32) {
    let v = shadow != 0;
    unsafe { set_window_feature(instance, window_ext::WindowCommand::SetShadow(v), |o| o.shadow = Some(v)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_skip_taskbar(instance: *mut RustinoWindow, skip: i32) {
    let v = skip != 0;
    unsafe { set_window_feature(instance, window_ext::WindowCommand::SetSkipTaskbar(v), |o| o.skip_taskbar = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_content_protection(instance: *mut RustinoWindow, enabled: i32) {
    let v = enabled != 0;
    unsafe {
        set_window_feature(instance, window_ext::WindowCommand::SetContentProtection(v), |o| {
            o.content_protection = v
        })
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_visible_on_all_workspaces(instance: *mut RustinoWindow, visible: i32) {
    let v = visible != 0;
    unsafe {
        set_window_feature(instance, window_ext::WindowCommand::SetVisibleOnAllWorkspaces(v), |o| {
            o.visible_on_all_workspaces = v
        })
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_closable(instance: *mut RustinoWindow, closable: i32) {
    let v = closable != 0;
    unsafe { set_window_feature(instance, window_ext::WindowCommand::SetClosable(v), |o| o.closable = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_minimizable(instance: *mut RustinoWindow, minimizable: i32) {
    let v = minimizable != 0;
    unsafe { set_window_feature(instance, window_ext::WindowCommand::SetMinimizable(v), |o| o.minimizable = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_maximizable(instance: *mut RustinoWindow, maximizable: i32) {
    let v = maximizable != 0;
    unsafe { set_window_feature(instance, window_ext::WindowCommand::SetMaximizable(v), |o| o.maximizable = v) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_always_on_bottom(instance: *mut RustinoWindow, on_bottom: i32) {
    let v = on_bottom != 0;
    unsafe {
        set_window_feature(instance, window_ext::WindowCommand::SetAlwaysOnBottom(v), |o| o.always_on_bottom = v)
    };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_ignore_cursor_events(instance: *mut RustinoWindow, ignore: i32) {
    let v = ignore != 0;
    unsafe {
        set_window_feature(instance, window_ext::WindowCommand::SetIgnoreCursorEvents(v), |o| {
            o.ignore_cursor_events = v
        })
    };
}

/// 0 default, 1 transparent, 2 overlay (see `MacTitleBarStyle`)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_mac_title_bar_style(instance: *mut RustinoWindow, style: i32) {
    let style = window_ext::MacTitleBarStyle::from_i32(style);
    unsafe {
        set_window_feature(instance, window_ext::WindowCommand::SetMacTitleBarStyle(style), |o| {
            o.mac_title_bar_style = style
        })
    };
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
        set_window_feature(instance, window_ext::WindowCommand::SetTrafficLightPosition(x, y), |o| {
            o.traffic_light_position = Some((x, y))
        })
    };
}

/// Linux: the `.desktop` file of the app, whose dock icon shows the badge and the progress
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_desktop_file_name(instance: *mut RustinoWindow, name: *const c_char) {
    let name = unsafe { util::cstr_to_string(name) };
    let command = window_ext::WindowCommand::SetDesktopFileName(name.clone());
    unsafe { set_window_feature(instance, command, |o| o.desktop_file_name = name) };
}

/// Moves the window with the mouse: call it while the left button is down.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_drag_window(instance: *mut RustinoWindow) {
    unsafe { run_window_feature(instance, window_ext::WindowCommand::DragWindow) };
}

/// Resizes the window with the mouse from an edge: 0 north, then clockwise to 7 north-west
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_drag_resize_window(instance: *mut RustinoWindow, direction: i32) {
    if let Some(direction) = window_ext::direction_from_i32(direction) {
        unsafe { run_window_feature(instance, window_ext::WindowCommand::DragResizeWindow(direction)) };
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

// ---------------------------------------------------------------------------
// Webview features (see webview_ext)
// ---------------------------------------------------------------------------

/// Changes a webview option before the window runs: 1 when applied, 0 once it started.
unsafe fn set_webview_option(
    instance: *mut RustinoWindow,
    store: impl FnOnce(&mut webview_ext::WebViewExtOptions),
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
    unsafe { post_webview(instance, webview_ext::print) };
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
    unsafe { post_webview(instance, webview_ext::open_devtools) };
}

/// Not supported on Windows
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_close_devtools(instance: *mut RustinoWindow) {
    unsafe { post_webview(instance, webview_ext::close_devtools) };
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
            inst.invoke(move |_, webview| webview_ext::cookies_json(webview, url.as_deref()))?
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
        let cookie = serde_json::from_str::<webview_ext::CookieData>(&json).ok()?.to_cookie();
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
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut webview_ext::DownloadResponse) -> i32,
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
    response: *mut webview_ext::DownloadResponse,
    path: *const c_char,
) {
    let _ = catch_unwind(|| {
        if let Some(r) = unsafe { response.as_mut() } {
            r.destination = unsafe { util::cstr_to_string(path) };
        }
    });
}

// ---------------------------------------------------------------------------
// Menus (queued until the window runs)
// ---------------------------------------------------------------------------

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
    unsafe { update_menu_item(instance, id, menu::MenuItemUpdate::Enabled(enabled != 0)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu_item_checked(
    instance: *mut RustinoWindow,
    id: *const c_char,
    checked: i32,
) {
    unsafe { update_menu_item(instance, id, menu::MenuItemUpdate::Checked(checked != 0)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu_item_text(
    instance: *mut RustinoWindow,
    id: *const c_char,
    text: *const c_char,
) {
    if let Some(text) = unsafe { util::cstr_to_string(text) } {
        unsafe { update_menu_item(instance, id, menu::MenuItemUpdate::Text(text)) };
    }
}

unsafe fn update_menu_item(
    instance: *mut RustinoWindow,
    id: *const c_char,
    update: menu::MenuItemUpdate,
) {
    let _ = catch_unwind(|| {
        if let Some(inst) = unsafe { instance.as_ref() } {
            if let Some(id) = unsafe { util::cstr_to_string(id) } {
                inst.send_command(RustinoCommand::UpdateMenuItem(id, update));
            }
        }
    });
}

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
                let params = commands::TrayParams {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_menu_event_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char, i32)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_menu_item_clicked = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_tray_icon_event_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, i32, i32, i32)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_tray_icon_clicked = handler) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_custom_scheme_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut window::SchemeResponse)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_custom_scheme = handler) };
}

// ---------------------------------------------------------------------------
// Splashscreen (standalone — no event loop required)
// ---------------------------------------------------------------------------

/// # Safety
/// `image_path` must be a valid null-terminated C string pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_splash_create(
    image_path: *const c_char,
    width: i32,
    height: i32,
) -> *mut splash::SplashWindow {
    catch_unwind(|| {
        let path = unsafe { util::cstr_to_string(image_path) }?;
        let w = width.max(1) as u32;
        let h = height.max(1) as u32;
        let splash = splash::SplashWindow::new(&path, w, h).ok()?;
        Some(Box::into_raw(Box::new(splash)))
    })
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `splash` must be a valid pointer returned from `rustino_splash_create`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_splash_close(splash: *mut splash::SplashWindow) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(s) = unsafe { splash.as_ref() } {
            s.close();
        }
    }));
}

/// # Safety
/// `splash` must be a valid pointer returned from `rustino_splash_create`, or null.
/// After calling this, the pointer is invalid and must not be used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_splash_dtor(splash: *mut splash::SplashWindow) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !splash.is_null() {
            unsafe {
                drop(Box::from_raw(splash));
            }
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_messages() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(payload.as_ref()), "The native window failed: boom");
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("bang"));
        assert_eq!(panic_message(payload.as_ref()), "The native window failed: bang");
        let payload: Box<dyn std::any::Any + Send> = Box::new(42);
        assert_eq!(panic_message(payload.as_ref()), "The native window failed: unknown error");
    }

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

    #[test]
    fn app_user_model_id_accepts_documented_form() {
        assert!(is_valid_app_user_model_id("Ivy.Tendril"));
        assert!(is_valid_app_user_model_id("Rustino"));
        assert!(is_valid_app_user_model_id(&"A".repeat(128)));
    }

    #[test]
    fn app_user_model_id_rejects_invalid_ids() {
        assert!(!is_valid_app_user_model_id(""));
        assert!(!is_valid_app_user_model_id("Ivy Tendril"));
        assert!(!is_valid_app_user_model_id("Ivy\tTendril"));
        assert!(!is_valid_app_user_model_id(r"Ivy\Tendril"));
        assert!(!is_valid_app_user_model_id(&"A".repeat(129)));
    }
}
