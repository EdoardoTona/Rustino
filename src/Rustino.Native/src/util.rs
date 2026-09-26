use std::ffi::{CStr, CString};
use std::os::raw::c_char;

pub unsafe fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(|s| s.to_string())
}

/// A JavaScript string literal with the text of `s`. JSON strings are valid JavaScript since
/// ES2019; U+2028 and U+2029 are escaped anyway for older engines.
pub fn js_string_literal(s: &str) -> String {
    serde_json::to_string(s)
        .unwrap_or_else(|_| "\"\"".to_string())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub fn free_cstring(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            drop(CString::from_raw(ptr));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn cstr_to_string_null_returns_none() {
        let result = unsafe { cstr_to_string(std::ptr::null()) };
        assert_eq!(result, None);
    }

    #[test]
    fn cstr_to_string_valid_utf8() {
        let s = CString::new("hello world").unwrap();
        let result = unsafe { cstr_to_string(s.as_ptr()) };
        assert_eq!(result, Some("hello world".to_string()));
    }

    #[test]
    fn cstr_to_string_empty_string() {
        let s = CString::new("").unwrap();
        let result = unsafe { cstr_to_string(s.as_ptr()) };
        assert_eq!(result, Some(String::new()));
    }

    #[test]
    fn cstr_to_string_unicode() {
        let s = CString::new("こんにちは 🌍").unwrap();
        let result = unsafe { cstr_to_string(s.as_ptr()) };
        assert_eq!(result, Some("こんにちは 🌍".to_string()));
    }

    #[test]
    fn js_string_literal_round_trips() {
        for text in ["", "plain", "quotes ' \" `", "back\\slash", "\0", "\u{0}1", "line\nbreak\r\t", "</script>", "\u{2028}\u{2029}", "🌍 ${x}"] {
            let literal = js_string_literal(text);
            assert!(!literal.contains('\u{2028}') && !literal.contains('\u{2029}'));
            assert_eq!(serde_json::from_str::<String>(&literal).unwrap(), text);
        }
    }

    #[test]
    fn js_string_literal_escapes_nul_without_octal() {
        // "\0" followed by a digit would be an octal escape, a syntax error in strict mode
        assert_eq!(js_string_literal("\u{0}1"), "\"\\u00001\"");
    }

    #[test]
    fn free_cstring_null_is_safe() {
        free_cstring(std::ptr::null_mut());
    }

    #[test]
    fn free_cstring_valid_pointer() {
        let s = CString::new("test").unwrap();
        let ptr = s.into_raw();
        free_cstring(ptr);
    }
}
