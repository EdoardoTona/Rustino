//! System notifications. They need no window.

use std::os::raw::c_char;
use std::panic::catch_unwind;

use crate::util;

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

#[cfg(test)]
mod tests {
    use super::*;

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
