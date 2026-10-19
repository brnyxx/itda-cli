//! URL validation and same-site checks shared by every layer.
//!
//! Derived from Hycli `src/net.rs` (`website_url`, `validate_url`,
//! `registrable_domain`, `same_site_origin`) and `src/runtime.rs` (`origin`,
//! `same_site_url`, `site_id_for`), Apache-2.0. Changes: moved out of the network
//! layer so pure checks carry no I/O dependency, and `registrable_domain` uses the
//! Public Suffix List instead of a hand-written list of 22 suffixes, so hosts such
//! as `a.github.io` and `b.github.io` are no longer treated as the same site.
use crate::error::{AppError, AppResult};
use crate::util;
use std::net::IpAddr;

/// Normalize a user-entered website address. Request and SiteSpec validation
/// still require an explicit scheme via [`validate_url`].
pub fn website_url(raw: &str) -> AppResult<url::Url> {
    let raw = raw.trim();
    let address = if raw.starts_with("//") {
        format!("https:{raw}")
    } else if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    validate_url(&address)
}

/// An absolute http(s) URL with a host, no embedded credentials and no
/// credential-named query parameters. The fragment is dropped.
pub fn validate_url(raw: &str) -> AppResult<url::Url> {
    if raw.len() > 8192 || raw.chars().any(char::is_control) {
        return Err(AppError::api("bad_url", 400));
    }
    let mut url = url::Url::parse(raw).map_err(|_| AppError::api("bad_url", 400))?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AppError::api("bad_url", 400));
    }
    if url.query_pairs().any(|(k, _)| util::secret_field(&k)) {
        return Err(AppError::api("bad_url", 400));
    }
    url.set_fragment(None);
    Ok(url)
}

/// The registrable domain (eTLD+1) of a host, or `None` for IP addresses,
/// `localhost` and bare public suffixes.
pub fn registrable_domain(host: &str) -> Option<String> {
    let host = host.trim_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.starts_with('[')
        || host.parse::<IpAddr>().is_ok()
        || !host.contains('.')
    {
        return None;
    }
    psl::domain_str(&host).map(str::to_owned)
}

/// True when two URLs share the exact origin, or the same registrable domain
/// with the same scheme and port.
pub fn same_site_origin(a: &url::Url, b: &url::Url) -> bool {
    if a.origin() == b.origin() {
        return true;
    }
    if a.scheme() != b.scheme() || a.port_or_known_default() != b.port_or_known_default() {
        return false;
    }
    let (Some(ha), Some(hb)) = (a.host_str(), b.host_str()) else {
        return false;
    };
    match (registrable_domain(ha), registrable_domain(hb)) {
        (Some(da), Some(db)) => da == db,
        _ => false,
    }
}

pub fn origin(raw: &str) -> AppResult<String> {
    Ok(validate_url(raw)?.origin().ascii_serialization())
}

pub fn same_site_url(a: &str, b: &str) -> AppResult<bool> {
    Ok(same_site_origin(&validate_url(a)?, &validate_url(b)?))
}

/// A stable, readable site id derived from an origin.
pub fn site_id_for(url: &url::Url) -> String {
    let host = url
        .host_str()
        .unwrap_or("website")
        .replace(['.', ':', '[', ']'], "-");
    format!(
        "site-{}-{}",
        host.trim_matches('-').chars().take(40).collect::<String>(),
        &util::hash(url.origin().ascii_serialization().as_bytes())[..8]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(raw: &str) -> url::Url {
        url::Url::parse(raw).unwrap()
    }

    #[test]
    fn website_input_normalizes_domains_without_weakening_request_validation() {
        assert_eq!(
            website_url("  blog.naver.com  ").unwrap().as_str(),
            "https://blog.naver.com/"
        );
        assert_eq!(
            website_url("//example.org/path").unwrap().as_str(),
            "https://example.org/path"
        );
        assert!(validate_url("example.org").is_err());
        for value in [
            "",
            "file:///etc/passwd",
            "https://user:password@example.org",
            "javascript:alert(1)",
            "https://example.org/?access_token=x",
        ] {
            assert!(website_url(value).is_err(), "{value}");
        }
        assert_eq!(
            validate_url("https://example.org/a#frag").unwrap().as_str(),
            "https://example.org/a"
        );
    }

    #[test]
    fn registrable_domain_follows_the_public_suffix_list() {
        assert_eq!(
            registrable_domain("www.example.co.kr").as_deref(),
            Some("example.co.kr")
        );
        assert_eq!(
            registrable_domain("api.example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            registrable_domain("shop.example.ne.jp").as_deref(),
            Some("example.ne.jp")
        );
        assert_eq!(
            registrable_domain("alice.github.io").as_deref(),
            Some("alice.github.io")
        );
        for none in ["localhost", "127.0.0.1", "[::1]", "com", ""] {
            assert_eq!(registrable_domain(none), None, "{none}");
        }
    }

    #[test]
    fn same_site_requires_scheme_port_and_registrable_domain() {
        assert!(same_site_origin(
            &u("https://example.com"),
            &u("https://api.example.com")
        ));
        assert!(!same_site_origin(
            &u("https://alice.github.io"),
            &u("https://bob.github.io")
        ));
        assert!(!same_site_origin(
            &u("http://example.com"),
            &u("https://example.com")
        ));
        assert!(!same_site_origin(
            &u("https://example.com"),
            &u("https://example.com:8443")
        ));
        assert!(!same_site_origin(
            &u("http://127.0.0.1:1"),
            &u("http://127.0.0.2:1")
        ));
    }

    #[test]
    fn site_ids_are_stable_and_valid() {
        let id = site_id_for(&u("https://www.example.co.kr/path"));
        assert!(id.starts_with("site-www-example-co-kr-"));
        assert!(util::valid_id(&id));
        assert_eq!(id, site_id_for(&u("https://www.example.co.kr/other")));
    }
}
