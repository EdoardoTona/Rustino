//! Custom URL schemes (e.g. `app://`), which the host serves: the webview asks, the host fills
//! the response from its handler.

use std::borrow::Cow;
use std::ffi::{CString, c_void};
use std::os::raw::c_char;
use std::panic::catch_unwind;

use crate::util;
use crate::window::{RustinoWindow, configure};

/// Pre-run only: 1 when applied, 0 once the window started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_add_custom_scheme(instance: *mut RustinoWindow, scheme: *const c_char) -> i32 {
    let scheme = unsafe { util::cstr_to_string(scheme) };
    unsafe { configure(instance, |s| s.config.custom_schemes.extend(scheme)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_custom_scheme_handler(
    instance: *mut RustinoWindow,
    handler: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut SchemeResponse)>,
) {
    unsafe { configure(instance, |s| s.callbacks.on_custom_scheme = handler) };
}

/// Called by the host from within the custom scheme handler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustino_set_scheme_response(
    response: *mut SchemeResponse,
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

/// WebView2 serves custom schemes as `http://<scheme>.localhost/`: maps these URLs back to
/// `<scheme>://localhost/`, the form custom scheme handlers receive. Only the exact host
/// `<scheme>.localhost` (with an optional port) maps: `http://app.evil.com/` stays as it is.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn revert_custom_scheme_workaround(url: &str, schemes: &[String]) -> String {
    let Some(rest) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) else {
        return url.to_string();
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => (host, Some(port)),
        Some(_) => return url.to_string(),
        None => (authority, None),
    };
    let host = host.to_ascii_lowercase();
    let Some(scheme) = host
        .strip_suffix(".localhost")
        .and_then(|name| schemes.iter().find(|scheme| scheme.eq_ignore_ascii_case(name)))
    else {
        return url.to_string();
    };
    match port {
        Some(port) => format!("{scheme}://localhost:{port}{path}"),
        None => format!("{scheme}://localhost{path}"),
    }
}

/// Filled by the host (via `rustino_set_scheme_response`) while the custom scheme callback runs.
#[derive(Default)]
pub struct SchemeResponse {
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
}

pub(crate) fn handle_custom_scheme(
    url: String,
    ctx: *mut std::ffi::c_void,
    cb: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char, *mut SchemeResponse),
) -> wry::http::Response<Cow<'static, [u8]>> {
    let mut response = SchemeResponse::default();
    if let Ok(cstr) = CString::new(url) {
        unsafe { cb(ctx, cstr.as_ptr(), &mut response) };
    }
    let builder = wry::http::Response::builder();
    match response.body {
        Some(body) => builder
            .header(
                wry::http::header::CONTENT_TYPE,
                response.content_type.unwrap_or_else(|| "application/octet-stream".into()),
            )
            .body(Cow::Owned(body)),
        None => builder.status(404).body(Cow::Borrowed(&[][..])),
    }
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    unsafe extern "C" fn respond_cb(
        _ctx: *mut std::ffi::c_void,
        url: *const std::ffi::c_char,
        response: *mut SchemeResponse,
    ) {
        let url = unsafe { CStr::from_ptr(url) }.to_str().unwrap();
        assert_eq!(url, "app://localhost/index.html");
        let body = b"<html></html>";
        unsafe {
            rustino_set_scheme_response(response, body.as_ptr(), body.len() as i32, c"text/html".as_ptr())
        };
    }

    unsafe extern "C" fn no_response_cb(
        _ctx: *mut std::ffi::c_void,
        _url: *const std::ffi::c_char,
        _response: *mut SchemeResponse,
    ) {
    }

    #[test]
    fn test_handle_custom_scheme_with_response() {
        let resp = handle_custom_scheme("app://localhost/index.html".to_string(), std::ptr::null_mut(), respond_cb);
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "text/html");
        assert_eq!(resp.body().as_ref(), b"<html></html>");
    }

    #[test]
    fn test_revert_custom_scheme_workaround() {
        let schemes = vec!["app".to_string()];
        let revert = |url: &str| revert_custom_scheme_workaround(url, &schemes);
        assert_eq!(revert("http://app.localhost/counter"), "app://localhost/counter");
        assert_eq!(revert("https://app.localhost/counter"), "app://localhost/counter");
        assert_eq!(revert("http://app.localhost"), "app://localhost");
        assert_eq!(revert("http://app.localhost:8080/a?b=c#d"), "app://localhost:8080/a?b=c#d");
        assert_eq!(revert("http://app.localhost?x=1"), "app://localhost?x=1");
        assert_eq!(revert("http://app.localhost#top"), "app://localhost#top");
        assert_eq!(revert("http://APP.localhost/"), "app://localhost/");
        assert_eq!(revert_custom_scheme_workaround("http://app.localhost/", &[]), "http://app.localhost/");
    }

    #[test]
    fn test_revert_custom_scheme_workaround_rejects_other_hosts() {
        let schemes = vec!["app".to_string()];
        let revert = |url: &str| revert_custom_scheme_workaround(url, &schemes);
        for url in [
            "https://example.com/",
            "http://app.evil.com/",
            "http://app.localhost.evil.com/",
            "http://app.localhostevil/",
            "http://evil.app.localhost/",
            "http://xapp.localhost/",
            "http://app.localhost:80x/",
            "http://app.localhost:/",
            "http://user@app.localhost/",
            "http://app.localhost@evil.com/",
            "ftp://app.localhost/",
            "app://localhost/",
            "https://example.com/?u=http://app.localhost/",
        ] {
            assert_eq!(revert(url), url);
        }
    }

    #[test]
    fn test_handle_custom_scheme_without_response_is_404() {
        let resp = handle_custom_scheme("app://localhost/missing".to_string(), std::ptr::null_mut(), no_response_cb);
        assert_eq!(resp.status(), 404);
        assert!(resp.body().is_empty());
    }
}
