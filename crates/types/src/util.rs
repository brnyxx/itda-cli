//! Small shared helpers: ids, hashing, private files and secret redaction.
//!
//! Derived from Hycli `src/util.rs` (Apache-2.0). Changes: temporary file prefix
//! renamed; otherwise unchanged so redaction behavior stays identical.
use crate::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

pub fn id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn private_dir(path: &Path) -> AppResult<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
/// Make an existing file readable and writable by its owner only. On
/// Windows the private directory's ACL covers it and nothing is done.
pub fn private_file(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
pub fn atomic_file(path: &Path, data: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::api("bad_request", 400))?;
    private_dir(parent)?;
    let temporary = parent.join(format!(".itda-{}", id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        {
            let directory = fs::File::open(parent)?;
            directory.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
pub fn read_bounded(path: &Path, limit: usize) -> AppResult<Vec<u8>> {
    use std::io::Read;
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(AppError::api("too_large", 413));
    }
    let mut result = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut result)?;
    if result.len() > limit {
        return Err(AppError::api("too_large", 413));
    }
    Ok(result)
}
/// The size and SHA-256 of a regular file, read in bounded chunks. A path
/// that is not a regular file (a directory, a pipe, a device) is refused.
pub fn file_digest(path: &Path) -> AppResult<(u64, String)> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(AppError::api("data_invalid", 400));
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((bytes, digest))
}
/// The media type a file name's extension says; a stream of bytes when
/// the extension says nothing itda knows.
pub fn media_type_of(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "json" => "application/json",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "txt" | "md" | "log" => "text/plain",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xls" => "application/vnd.ms-excel",
        "doc" => "application/msword",
        "hwp" => "application/x-hwp",
        _ => "application/octet-stream",
    }
}
pub fn plain(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
pub fn constant_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// This is applied before results enter a model, log, export, or dashboard response.
pub fn redact(value: &mut serde_json::Value, secrets: &[String]) {
    match value {
        serde_json::Value::Object(object) => {
            if object
                .get("name")
                .or_else(|| object.get("key"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(secret_field)
                && let Some(value) = object.get_mut("value")
            {
                *value = serde_json::Value::String("[private]".into());
            }
            for (key, child) in object.iter_mut() {
                if secret_field(key) {
                    *child = serde_json::Value::String("[private]".into());
                } else {
                    redact(child, secrets);
                }
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                redact(child, secrets);
            }
        }
        serde_json::Value::String(text) => {
            for secret in secrets {
                // A preference such as "en" or "1" must not corrupt every word or handle.
                // Short values are still removed when they are the complete scalar;
                // credential-named fields are always redacted above.
                if secret.chars().count() >= 8 {
                    *text = text.replace(secret, "[private]");
                } else if !secret.is_empty() {
                    let word = |c: char| c.is_alphanumeric() || c == '_';
                    let mut cleaned = String::new();
                    let mut copied = 0;
                    for (start, _) in text.match_indices(secret) {
                        let end = start + secret.len();
                        if text[..start].chars().next_back().is_none_or(|c| !word(c))
                            && text[end..].chars().next().is_none_or(|c| !word(c))
                        {
                            cleaned.push_str(&text[copied..start]);
                            cleaned.push_str("[private]");
                            copied = end;
                        }
                    }
                    cleaned.push_str(&text[copied..]);
                    *text = cleaned;
                }
            }
            *text = redact_text(text);
        }
        _ => {}
    }
}
pub fn secret_field(key: &str) -> bool {
    let lower = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "tokens",
        "cookies",
        "session",
        "auth",
        "token",
        "authtoken",
        "csrf",
        "xsrf",
        "credential",
        "credentials",
        "password",
        "passwd",
        "authorization",
        "cookie",
        "setcookie",
        "accesstoken",
        "refreshtoken",
        "idtoken",
        "csrftoken",
        "xsrftoken",
        "apikey",
        "secret",
        "clientsecret",
        "sessiontoken",
        "sessionid",
        "bearer",
    ]
    .iter()
    .any(|word| lower == *word || lower.ends_with(word))
}
pub fn redact_text(text: &str) -> String {
    use std::sync::LazyLock;
    static TOKENS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?i)(?:bearer\s+[A-Za-z0-9._~+/=-]+|(?:sk-|xai-)[A-Za-z0-9_-]{16,}|eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)"#).expect("constant pattern")
    });
    static ASSIGNMENTS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?i)((?:[a-z_-]*token|api[_-]?key|csrf|xsrf|password|passwd|secret|authorization|cookie|session[_-]?id)["']?\s*[:=]\s*)["']?[^\s"'<>,;&}]+["']?"#).expect("constant pattern")
    });
    ASSIGNMENTS
        .replace_all(&TOKENS.replace_all(text, "[private]"), "${1}[private]")
        .into_owned()
}

/// Model output may be a typed tool proposal or SiteSpec. Sanitize string contents
/// before serialization so a textual header example cannot consume JSON quotes.
/// Schema keys such as `auth` contain references, not credential values.
pub fn redact_generated_text(text: &str) -> String {
    fn strings(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => *text = redact_text(text),
            serde_json::Value::Array(values) => values.iter_mut().for_each(strings),
            serde_json::Value::Object(values) => values.values_mut().for_each(strings),
            _ => {}
        }
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(mut value) => {
            strings(&mut value);
            value.to_string()
        }
        Err(_) => redact_text(text),
    }
}

/// The claims of a JSON Web Token (`header.payload.signature`, each part
/// base64url), when `token` is one. Nothing is checked: the signature is the
/// site's business. This only reads when a sign-in ends and who it is for.
pub fn jwt_claims(token: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut parts = token.trim().split('.');
    let (header, payload, signature) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || header.is_empty() || signature.is_empty() {
        return None;
    }
    let header: serde_json::Value = serde_json::from_slice(&base64url(header)?).ok()?;
    if !header.is_object() {
        return None;
    }
    match serde_json::from_slice(&base64url(payload)?).ok()? {
        serde_json::Value::Object(claims) => Some(claims),
        _ => None,
    }
}

/// When a JSON Web Token stops being accepted: its `exp` claim, in seconds
/// since the epoch. `None` for a token without one, or a value that is not one.
pub fn jwt_expiry(token: &str) -> Option<i64> {
    let claims = jwt_claims(token)?;
    let exp = claims.get("exp")?;
    exp.as_i64()
        .or_else(|| exp.as_f64().map(|f| f as i64))
        .filter(|exp| *exp > 0)
}

/// Who a JSON Web Token says it is for, as a site would show it: the first of
/// `email`, `preferred_username`, `name` and `sub` that is a string.
pub fn jwt_identity(token: &str) -> Option<String> {
    let claims = jwt_claims(token)?;
    ["email", "preferred_username", "name", "sub"]
        .iter()
        .find_map(|claim| claims.get(*claim)?.as_str().map(str::to_owned))
        .filter(|value| !value.trim().is_empty())
}

/// A base64url text (RFC 4648 section 5, with or without padding) as bytes.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A date and time as `2026-10-24T07:00:00Z`, from seconds since the epoch.
pub fn iso_date(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// The candidate closest to `wanted` by edit distance, when it is close
/// enough to be a slip: within half the length of the longer name, or one
/// a prefix of the other. Names are compared without case.
pub fn nearest<'a>(wanted: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let wanted = wanted.to_ascii_lowercase();
    if wanted.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        let lower = candidate.to_ascii_lowercase();
        if lower == wanted {
            return Some(candidate);
        }
        let prefix = lower.starts_with(&wanted) || wanted.starts_with(&lower);
        let distance = edit_distance(&wanted, &lower);
        let allowed = wanted.len().max(lower.len()) / 2;
        if (prefix || distance <= allowed.max(1)) && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current.push(
                (previous[j] + cost)
                    .min(previous[j + 1] + 1)
                    .min(current[j] + 1),
            );
        }
        previous = current;
    }
    previous[b.len()]
}

pub fn file_lock(path: &Path, nonblocking: bool) -> AppResult<fs::File> {
    if let Some(parent) = path.parent() {
        private_dir(parent)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    if nonblocking {
        file.try_lock()
            .map_err(|_| AppError::api("conflict", 409))?;
    } else {
        file.lock()?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";

    #[test]
    fn a_jwt_gives_up_its_expiry_and_identity_without_being_trusted() {
        let token = format!(
            "{HEADER}.eyJzdWIiOiJ1LTEiLCJlbWFpbCI6Im1lQGV4YW1wbGUuY29tIiwiZXhwIjoxNzAwMDAwMDAwfQ.sig"
        );
        assert_eq!(jwt_expiry(&token), Some(1_700_000_000));
        assert_eq!(jwt_identity(&token).as_deref(), Some("me@example.com"));
        assert_eq!(iso_date(1_700_000_000), "2023-11-14T22:13:20Z");
        // No exp: unknown. A name alone still says who.
        let named = format!("{HEADER}.eyJuYW1lIjoiS2ltIn0.sig");
        assert_eq!(jwt_expiry(&named), None);
        assert_eq!(jwt_identity(&named).as_deref(), Some("Kim"));
        // Not a token: two parts, a payload that is a list, a header that is not JSON.
        assert_eq!(jwt_claims(&format!("{HEADER}.eyJuYW1lIjoiS2ltIn0")), None);
        assert_eq!(jwt_claims(&format!("{HEADER}.WyJ4Il0.sig")), None);
        assert_eq!(jwt_claims("abc.def.ghi"), None);
        assert_eq!(jwt_claims("3f9a8c7b2e1d4a6f"), None);
        // Padding is accepted and ignored.
        assert_eq!(
            base64url("eyJuYW1lIjoiS2ltIn0=").as_deref(),
            Some(br#"{"name":"Kim"}"#.as_slice())
        );
        assert_eq!(base64url("not base64!"), None);
    }

    #[test]
    fn ids_are_lowercase_slugs() {
        assert!(valid_id("site-1"));
        for bad in ["", "1site", "Site", "site_1", &"a".repeat(65)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn secret_fields_and_values_are_redacted() {
        let mut value = serde_json::json!({"token":"opaque-token-1234","authToken":"opaque-auth-5678","csrf":"csrf-opaque-value","cookies":[{"name":"sid","value":"unseen-cookie-value"}],"headers":[{"name":"Authorization","value":"opaque-header"}],"echo":"short x credential","useful":{"name":"Mina","count":3}});
        redact(&mut value, &["x".into()]);
        let text = value.to_string();
        for private in [
            "opaque-token-1234",
            "opaque-auth-5678",
            "csrf-opaque-value",
            "unseen-cookie-value",
            "opaque-header",
            " x ",
        ] {
            assert!(!text.contains(private), "{private} in {text}");
        }
        assert!(text.contains("Mina"));
        assert_eq!(
            redact_text("Authorization: Bearer abc.def"),
            "Authorization: [private]"
        );
    }

    #[test]
    fn the_nearest_name_is_a_slip_away() {
        let names = ["list-notices", "create-notice", "search"];
        assert_eq!(nearest("list-notice", names), Some("list-notices"));
        assert_eq!(nearest("List_Notices", names), Some("list-notices"));
        assert_eq!(nearest("serach", names), Some("search"));
        assert_eq!(nearest("creat", names), Some("create-notice"));
        assert_eq!(nearest("delete-everything", names), None);
        assert_eq!(nearest("", names), None);
    }

    #[test]
    fn constant_eq_compares_whole_strings() {
        assert!(constant_eq("abc", "abc"));
        assert!(!constant_eq("abc", "abd"));
        assert!(!constant_eq("abc", "ab"));
    }
}
