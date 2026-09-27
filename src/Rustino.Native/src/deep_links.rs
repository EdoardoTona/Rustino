use std::ffi::c_char;
use std::panic::catch_unwind;

use crate::commands::RustinoCommand;
use crate::window::RustinoWindow;
use crate::window_ext::WindowCommand;

/// Delivers a JSON array of URL strings through the window's UrlsOpened callback.
/// The URLs are copied, bounded and queued on the event loop before this function returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_deliver_urls(instance: *mut RustinoWindow, json_urls: *const c_char) -> i32 {
    let Some(instance) = (unsafe { instance.as_ref() }) else {
        return 0;
    };

    catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(json) = (unsafe { crate::util::cstr_to_string(json_urls) }) else {
            return 0;
        };
        if json.len() > 1024 * 1024 {
            return 0;
        }
        let Ok(urls) = serde_json::from_str::<Vec<String>>(&json) else {
            return 0;
        };
        if urls.is_empty()
            || urls.len() > 128
            || urls.iter().any(|url| {
                url.is_empty()
                    || url.len() > 32 * 1024
                    || url.chars().any(char::is_control)
            })
        {
            return 0;
        }

        instance.send_command(RustinoCommand::Window(WindowCommand::DeliverUrls(urls))) as i32
    }))
    .unwrap_or(0)
}
