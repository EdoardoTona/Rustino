use std::ffi::c_char;
use std::panic::catch_unwind;

/// Opens a validated http, https or mailto URL in the system's default application.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_open_external(url: *const c_char) -> i32 {
    catch_unwind(|| {
        let Some(url) = (unsafe { crate::util::cstr_to_string(url) }) else {
            return 0;
        };
        if !is_allowed_url(&url) {
            return 0;
        }
        if open_external(&url) { 1 } else { 0 }
    })
    .unwrap_or(0)
}

fn is_allowed_url(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 32 * 1024
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" if rest.starts_with("//") => {}
        "mailto" if !rest.is_empty() => {}
        _ => return false,
    }

    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    match scheme.as_str() {
        "http" | "https" => parsed.host().is_some(),
        "mailto" => !parsed.path().is_empty(),
        _ => false,
    }
}

#[cfg(target_os = "windows")]
fn open_external(url: &str) -> bool {
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{PCWSTR, w};

    let url = url.to_owned();
    std::thread::spawn(move || {
        if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_err() {
            return false;
        }
        let url_wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            ShellExecuteW(None, w!("open"), PCWSTR(url_wide.as_ptr()), None, None, SW_SHOWNORMAL)
        };
        unsafe { CoUninitialize() };
        (result.0 as isize) > 32
    })
    .join()
    .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn open_external(url: &str) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    let string = NSString::from_str(url);
    let Some(url) = NSURL::URLWithString(&string) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

#[cfg(target_os = "linux")]
fn open_external(url: &str) -> bool {
    if gtk::gio::AppInfo::launch_default_for_uri(url, None::<&gtk::gio::AppLaunchContext>).is_ok() {
        return true;
    }
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .is_ok()
}
