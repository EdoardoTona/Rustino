use std::ffi::c_char;
use std::panic::catch_unwind;

use tao::window::Window;

use crate::window::RustinoWindow;

/// Shows, restores and focuses the window on its event loop thread.
///
/// `activation_token` is used on Linux to pass launcher activation metadata to GTK. The value is
/// copied before this function returns. Returns 1 when the operation ran or was queued, and 0 if
/// the instance is null, closed, or a native panic was caught.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_activate(
    instance: *mut RustinoWindow,
    activation_token: *const c_char,
) -> i32 {
    let Some(instance) = (unsafe { instance.as_ref() }) else {
        return 0;
    };

    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let token = unsafe { crate::util::cstr_to_string(activation_token) };
        instance.post(move |window, _webview| activate(window, token.as_deref()))
    }))
    .unwrap_or(false) as i32
}

fn activate(window: &Window, _activation_token: Option<&str>) {
    #[cfg(not(target_os = "linux"))]
    let _ = _activation_token;
    window.set_visible(true);
    if window.is_minimized() {
        window.set_minimized(false);
    }

    #[cfg(target_os = "windows")]
    activate_windows(window);

    #[cfg(target_os = "macos")]
    activate_macos(window);

    #[cfg(target_os = "linux")]
    activate_linux(window, _activation_token);
}

#[cfg(target_os = "windows")]
fn activate_windows(window: &Window) {
    use tao::platform::windows::WindowExtWindows;
    use tao::window::UserAttentionType;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

    let focused = unsafe { SetForegroundWindow(HWND(window.hwnd() as _)).as_bool() };
    if !focused {
        window.request_user_attention(Some(UserAttentionType::Informational));
    }
}

#[cfg(target_os = "macos")]
fn activate_macos(window: &Window) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use tao::platform::macos::WindowExtMacOS;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(main_thread);
    let ns_window = window.ns_window().cast::<objc2::runtime::AnyObject>();
    unsafe {
        let minimized: bool = objc2::msg_send![ns_window, isMiniaturized];
        if minimized {
            let _: () = objc2::msg_send![
                ns_window,
                deminiaturize: std::ptr::null_mut::<objc2::runtime::AnyObject>()
            ];
        }
        let _: () = objc2::msg_send![
            ns_window,
            makeKeyAndOrderFront: std::ptr::null_mut::<objc2::runtime::AnyObject>()
        ];
    }
    app.unhide(None);
    app.activate();
}

#[cfg(target_os = "linux")]
fn activate_linux(window: &Window, activation_token: Option<&str>) {
    use gtk::prelude::GtkWindowExt;
    use tao::platform::unix::WindowExtUnix;

    let gtk_window = window.gtk_window();
    if let Some(token) = activation_token {
        gtk_window.set_startup_id(token);
    }
    gtk_window.deiconify();
    gtk_window.present_with_time(activation_token.map(startup_id_timestamp).unwrap_or(0));
}

#[cfg(target_os = "linux")]
fn startup_id_timestamp(startup_id: &str) -> u32 {
    startup_id
        .rsplit_once("_TIME")
        .and_then(|(_, value)| value.parse::<u32>().ok())
        .unwrap_or(0)
}
