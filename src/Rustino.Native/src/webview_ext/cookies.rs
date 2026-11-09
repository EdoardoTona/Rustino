//! The cookies of the webview, as JSON for the host.

use serde::{Deserialize, Serialize};
use wry::cookie::{Cookie, SameSite};

/// A cookie as the host sees it (JSON, camelCase).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CookieData {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    /// Unix time in seconds; none for session cookies
    #[serde(default)]
    pub expires: Option<i64>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// "Strict", "Lax" or "None"
    #[serde(default)]
    pub same_site: Option<String>,
}

impl From<&Cookie<'_>> for CookieData {
    fn from(cookie: &Cookie<'_>) -> Self {
        Self {
            name: cookie.name().to_string(),
            value: cookie.value().to_string(),
            domain: cookie.domain().map(str::to_string),
            path: cookie.path().map(str::to_string),
            expires: cookie.expires_datetime().map(|t| t.unix_timestamp()),
            secure: cookie.secure().unwrap_or(false),
            http_only: cookie.http_only().unwrap_or(false),
            same_site: cookie.same_site().map(|s| s.to_string()),
        }
    }
}

impl CookieData {
    pub fn to_cookie(&self) -> Cookie<'static> {
        let mut builder = Cookie::build((self.name.clone(), self.value.clone()))
            .secure(self.secure)
            .http_only(self.http_only);
        if let Some(domain) = &self.domain {
            builder = builder.domain(domain.clone());
        }
        if let Some(path) = &self.path {
            builder = builder.path(path.clone());
        }
        if let Some(expires) = self.expires.and_then(|t| wry::cookie::time::OffsetDateTime::from_unix_timestamp(t).ok()) {
            builder = builder.expires(expires);
        }
        let same_site = match self.same_site.as_deref() {
            Some(s) if s.eq_ignore_ascii_case("strict") => Some(SameSite::Strict),
            Some(s) if s.eq_ignore_ascii_case("lax") => Some(SameSite::Lax),
            Some(s) if s.eq_ignore_ascii_case("none") => Some(SameSite::None),
            _ => None,
        };
        if let Some(same_site) = same_site {
            builder = builder.same_site(same_site);
        }
        builder.build()
    }
}

/// JSON array of the webview's cookies, all of them or those sent to `url`.
pub fn cookies_json(webview: &wry::WebView, url: Option<&str>) -> Option<String> {
    let cookies = match url {
        Some(url) => webview.cookies_for_url(url),
        None => webview.cookies(),
    }
    .ok()?;
    let data: Vec<CookieData> = cookies.iter().map(CookieData::from).collect();
    serde_json::to_string(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_round_trip() {
        let json = r#"{"name":"session","value":"abc","domain":"example.com","path":"/","expires":1893456000,"secure":true,"httpOnly":true,"sameSite":"Lax"}"#;
        let data: CookieData = serde_json::from_str(json).unwrap();
        let cookie = data.to_cookie();
        assert_eq!(cookie.name(), "session");
        assert_eq!(cookie.domain(), Some("example.com"));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.expires_datetime().unwrap().unix_timestamp(), 1893456000);
        assert_eq!(CookieData::from(&cookie), data);
    }

    #[test]
    fn cookie_defaults() {
        let data: CookieData = serde_json::from_str(r#"{"name":"a","value":"b"}"#).unwrap();
        let cookie = data.to_cookie();
        assert_eq!(cookie.secure(), Some(false));
        assert!(cookie.expires_datetime().is_none());
        assert!(cookie.same_site().is_none());
    }
}
