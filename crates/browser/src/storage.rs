//! What the signed-in page keeps in the browser's own storage: a single-page
//! app often has no session cookie at all and keeps a token in
//! `localStorage` instead, which its scripts send as `Authorization: Bearer`.
//!
//! itda reads that storage once, over the DevTools protocol, from the window
//! the person opened and signed in to, for that site's origin only. It never
//! reads another browser's profile on disk and never puts anything into a
//! page. A value is kept only when the recorded traffic shows the site's own
//! scripts sending it as a header; otherwise it is left where it was.
use itda_types::error::{AppError, AppResult};
use serde_json::{Value, json};

/// One item of a page's `localStorage` or `sessionStorage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageItem {
    pub key: String,
    pub value: String,
    /// `localStorage` when true, `sessionStorage` when false.
    pub local: bool,
}

/// A session a site keeps in storage and sends as a header: the storage
/// key it is under, the header the site's scripts put it in, the prefix in
/// front of it there (`Bearer `) and the value itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageSession {
    pub key: String,
    pub local: bool,
    pub header: String,
    pub prefix: String,
    pub value: String,
}

/// Sends one DevTools command, to the browser (`None`) or to one attached
/// page session, and returns its result.
pub(crate) type Send<'a> = dyn FnMut(Option<&str>, &str, Value) -> AppResult<Value> + 'a;

/// The page session already attached to a target, by target id, for a caller
/// that attaches to every page itself.
pub(crate) type SessionFor<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// The `localStorage` and `sessionStorage` items of `site`'s origin, read
/// from a page open at that origin. `session_for` gives the session already
/// attached to a target, for a caller that attaches to every page itself;
/// without it a page session is attached for the read and detached after.
/// No page at the origin, or storage the browser will not give, is empty.
///
/// Methods, checked against the protocol the browser serves at
/// `/json/protocol` (Chrome 155, protocol 1.3): `Target.getTargets` returns
/// `targetInfos[]` with `targetId`, `type` and `url`; `Target.attachToTarget
/// {targetId, flatten}` returns `sessionId`; `DOMStorage.getDOMStorageItems
/// {storageId: {securityOrigin, isLocalStorage}}` returns `entries`, each a
/// `[key, value]` pair; `Target.detachFromTarget {sessionId}`.
pub(crate) fn read(
    send: &mut Send<'_>,
    site: &url::Url,
    session_for: Option<&SessionFor<'_>>,
) -> AppResult<Vec<StorageItem>> {
    let origin = site.origin();
    if !origin.is_tuple() {
        return Ok(Vec::new());
    }
    let targets = send(None, "Target.getTargets", json!({}))?;
    let page = targets["targetInfos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|target| target["type"].as_str() == Some("page"))
        .find(|target| {
            target["url"]
                .as_str()
                .and_then(|url| url::Url::parse(url).ok())
                .is_some_and(|url| url.origin() == origin)
        })
        .and_then(|target| target["targetId"].as_str().map(str::to_owned));
    let Some(target) = page else {
        return Ok(Vec::new());
    };
    let (session, attached) = match session_for {
        Some(known) => match known(&target) {
            Some(session) => (session, false),
            None => return Ok(Vec::new()),
        },
        None => {
            let reply = send(
                None,
                "Target.attachToTarget",
                json!({"targetId": target, "flatten": true}),
            )?;
            let session = reply["sessionId"]
                .as_str()
                .ok_or_else(|| AppError::api("browser_unavailable", 502))?
                .to_owned();
            (session, true)
        }
    };
    let security_origin = origin.ascii_serialization();
    let mut items = Vec::new();
    for local in [true, false] {
        let reply = send(
            Some(&session),
            "DOMStorage.getDOMStorageItems",
            json!({"storageId": {"securityOrigin": security_origin, "isLocalStorage": local}}),
        );
        // Storage the page never touched may be reported as an error rather
        // than as empty; either way there is nothing in it.
        if let Ok(reply) = reply {
            items.extend(parse_items(&reply, local));
        }
    }
    if attached {
        let _ = send(
            None,
            "Target.detachFromTarget",
            json!({"sessionId": session}),
        );
    }
    Ok(items)
}

/// The items in a `DOMStorage.getDOMStorageItems` result: `entries`, each a
/// `[key, value]` pair of strings.
pub(crate) fn parse_items(reply: &Value, local: bool) -> Vec<StorageItem> {
    reply["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let pair = entry.as_array()?;
            Some(StorageItem {
                key: pair.first()?.as_str()?.to_owned(),
                value: pair.get(1)?.as_str()?.to_owned(),
                local,
            })
        })
        .collect()
}

/// Shortest value that counts as a token in storage.
const TOKEN_LENGTH: usize = 16;

/// Whether a storage item looks like a session value: a JSON Web Token, or
/// a long opaque value under a key named for one (`token`, `access_token`,
/// `id_token`, `auth`, `session`, `jwt`). Only the shape is read.
pub fn looks_like_token(item: &StorageItem) -> bool {
    let value = item.value.trim();
    if value.len() < TOKEN_LENGTH || value.chars().any(char::is_whitespace) {
        return false;
    }
    if itda_types::util::jwt_claims(value).is_some() {
        return true;
    }
    let key = item.key.to_ascii_lowercase();
    ["token", "auth", "session", "jwt"]
        .iter()
        .any(|word| key.contains(word))
}

/// The storage item the recorded traffic sent as a header to `origin`: the
/// evidence that it is the session. The latest request that sent one wins;
/// `localStorage` wins over `sessionStorage` when both hold the value. The
/// header is `Authorization` with its scheme as the prefix (`Bearer `), or a
/// custom header sent as is. `None` when no request sent a stored value.
pub fn bearer_session(
    har: &Value,
    items: &[StorageItem],
    origin: &url::Origin,
) -> Option<StorageSession> {
    let candidates: Vec<&StorageItem> = items
        .iter()
        .filter(|item| item.value.trim().len() >= TOKEN_LENGTH)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let entries = har["log"]["entries"].as_array()?;
    entries.iter().rev().find_map(|entry| {
        let url = url::Url::parse(entry["request"]["url"].as_str()?).ok()?;
        if url.origin() != *origin {
            return None;
        }
        entry["request"]["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|header| {
                let name = header["name"].as_str()?;
                let value = header["value"].as_str()?.trim();
                let (prefix, token) = split_scheme(name, value);
                let item = candidates
                    .iter()
                    .filter(|item| item.value.trim() == token)
                    .max_by_key(|item| item.local)?;
                let named = {
                    let lower = name.to_ascii_lowercase();
                    lower == "authorization"
                        || ["token", "auth", "key"]
                            .iter()
                            .any(|word| lower.contains(word))
                };
                if !named && !looks_like_token(item) {
                    return None;
                }
                Some(StorageSession {
                    key: item.key.clone(),
                    local: item.local,
                    header: name.to_owned(),
                    prefix,
                    value: item.value.trim().to_owned(),
                })
            })
    })
}

/// An `Authorization` value as its scheme prefix (`Bearer `) and the token
/// after it; any other header is sent whole.
fn split_scheme(name: &str, value: &str) -> (String, String) {
    if name.eq_ignore_ascii_case("authorization")
        && let Some((scheme, token)) = value.split_once(' ')
        && !scheme.is_empty()
        && !token.trim().is_empty()
    {
        return (format!("{scheme} "), token.trim().to_owned());
    }
    (String::new(), value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const JWT: &str =
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1LTEiLCJleHAiOjQxMDI0NDQ4MDB9.c2ln";

    fn item(key: &str, value: &str, local: bool) -> StorageItem {
        StorageItem {
            key: key.into(),
            value: value.into(),
            local,
        }
    }

    #[test]
    fn storage_entries_are_key_value_pairs() {
        let reply = json!({"entries": [["token", JWT], ["theme", "dark"], ["odd"], [1, 2]]});
        let items = parse_items(&reply, true);
        assert_eq!(
            items,
            [item("token", JWT, true), item("theme", "dark", true)]
        );
    }

    #[test]
    fn a_token_is_a_jwt_or_a_long_value_under_a_token_name() {
        assert!(looks_like_token(&item("anything", JWT, true)));
        assert!(looks_like_token(&item(
            "access_token",
            "3f9a8c7b2e1d4a6f3f9a8c7b2e1d4a6f",
            true
        )));
        assert!(!looks_like_token(&item("theme", "dark", true)));
        assert!(!looks_like_token(&item("token", "short", true)));
        assert!(!looks_like_token(&item(
            "note",
            "a sentence the person typed into the app",
            false
        )));
    }

    #[test]
    fn the_item_a_request_sent_as_a_bearer_header_is_the_session() {
        let origin = url::Url::parse("https://app.example").unwrap().origin();
        let har = json!({"log": {"entries": [
            {"request": {"url": "https://app.example/api/login", "headers": []}},
            {"request": {"url": "https://other.example/x", "headers": [{"name": "Authorization", "value": "Bearer elsewhere-elsewhere-elsewhere"}]}},
            {"request": {"url": "https://app.example/api/items", "headers": [
                {"name": "Accept", "value": "application/json"},
                {"name": "Authorization", "value": format!("Bearer {JWT}")},
            ]}},
        ]}});
        let items = [
            item("theme", "dark", true),
            item("token", JWT, false),
            item("token", JWT, true),
            item("elsewhere", "elsewhere-elsewhere-elsewhere", true),
        ];
        let found = bearer_session(&har, &items, &origin).unwrap();
        assert_eq!(found.key, "token");
        assert!(found.local, "localStorage wins over sessionStorage");
        assert_eq!(found.header, "Authorization");
        assert_eq!(found.prefix, "Bearer ");
        assert_eq!(found.value, JWT);
        // A value no same-origin request sent is not a session.
        assert_eq!(bearer_session(&har, &items[..1], &origin), None);
        assert_eq!(bearer_session(&har, &items[3..], &origin), None);
    }

    #[test]
    fn a_custom_header_counts_when_named_for_a_token_or_the_value_looks_like_one() {
        let origin = url::Url::parse("https://app.example").unwrap().origin();
        let opaque = "3f9a8c7b2e1d4a6f3f9a8c7b2e1d4a6f";
        let har = json!({"log": {"entries": [
            {"request": {"url": "https://app.example/api/items", "headers": [
                {"name": "X-Auth-Token", "value": opaque},
                {"name": "X-Trace", "value": "trace-trace-trace-trace"},
            ]}},
        ]}});
        let items = [
            item("session", opaque, true),
            item("trace", "trace-trace-trace-trace", true),
        ];
        let found = bearer_session(&har, &items, &origin).unwrap();
        assert_eq!(
            (found.header.as_str(), found.prefix.as_str()),
            ("X-Auth-Token", "")
        );
        assert_eq!(found.key, "session");
        assert_eq!(bearer_session(&har, &items[1..], &origin), None);
    }
}
