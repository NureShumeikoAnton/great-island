//! A connector's network request and answer, and which addresses it may reach: https, a host the
//! connector declared, nothing else. The request itself is made by the island, never by the app.

use super::Service;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
        }
    }
}

/// The host of an https address; `None` for anything else, a port or user info included: a
/// declared host means the standard port.
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..end];
    if host.is_empty() || host.contains([':', '@']) {
        return None;
    }
    Some(host)
}

pub fn allowed(service: &Service, url: &str) -> bool {
    host_of(url).is_some_and(|host| {
        service
            .hosts
            .iter()
            .any(|declared| declared.eq_ignore_ascii_case(host))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather() -> Service {
        Service {
            provider: "Open-Meteo",
            hosts: &["api.open-meteo.com"],
            site: "",
            sends: Vec::new(),
            note: String::new(),
        }
    }

    #[test]
    fn a_declared_host_over_https_is_allowed() {
        let s = weather();
        assert!(allowed(&s, "https://api.open-meteo.com/v1/forecast?lat=1"));
        assert!(allowed(&s, "https://API.open-meteo.com"));
    }

    #[test]
    fn anything_else_is_refused() {
        let s = weather();
        for url in [
            "http://api.open-meteo.com/v1",
            "https://open-meteo.com/",
            "https://api.open-meteo.com.evil.example/",
            "https://evil.example/?https://api.open-meteo.com",
            "https://api.open-meteo.com:8443/",
            "https://user@api.open-meteo.com/",
            "https:///v1",
            "file:///C:/Windows",
        ] {
            assert!(!allowed(&s, url), "{url}");
        }
    }
}
