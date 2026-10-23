//! Turning pages. A read that declares `pagination` gets its next marker
//! from the answer: a cursor the site handed out, the page after this one,
//! or the offset past the records returned. The marker is kept in the
//! handle store and the agent gets `meta.next`, an opaque handle bound to
//! this exact call. `next` on the following call puts the marker back in
//! the declared input; the raw marker is never shown to the agent and is
//! removed from the data it appears in.
use itda_policy::{ExecutionRequest, operation_effect, safe_read_url};
use itda_spec::{Effect, Operation, Pagination, ResponseLike, Spec};
use itda_types::{error::AppResult, json::resolve_pointer, url::validate_url, util};
use serde_json::Value;
use std::collections::BTreeMap;

/// The longest cursor a site may hand out.
const MAX_CURSOR: usize = 4096;

/// What binds a handle to one call: everything an approval is bound to,
/// with the marker input left out of the inputs.
pub(crate) fn binding(
    spec: &Spec,
    op: &Operation,
    account_id: &str,
    args: &BTreeMap<String, String>,
    credential_hash: &str,
) -> AppResult<String> {
    let input = op.pagination_input().unwrap_or_default();
    ExecutionRequest {
        site_id: spec.site.name.clone(),
        action_id: op.name.clone(),
        account_id: account_id.to_owned(),
        inputs: args
            .iter()
            .filter(|(name, _)| name.as_str() != input)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        spec_hash: util::hash(&serde_json::to_vec(spec)?),
        credential_hash: credential_hash.to_owned(),
        file_hash: String::new(),
    }
    .digest()
}

/// A marker a site handed out that is safe to send back: short enough,
/// printable, not an address, a key or a document.
pub(crate) fn acceptable(pagination: &Pagination, value: &str) -> bool {
    if pagination.style != "cursor" {
        return value.parse::<u64>().is_ok();
    }
    !value.is_empty()
        && value.len() <= MAX_CURSOR
        && !value.chars().any(char::is_control)
        && !value.contains("://")
        && !value.contains("-----BEGIN")
}

/// The marker for the page after this answer, when there is one and it
/// still describes a read of the same site.
pub(crate) fn next_marker(
    spec: &Spec,
    op: &Operation,
    pagination: &Pagination,
    response: &impl ResponseLike,
    data: &Value,
    args: &BTreeMap<String, String>,
) -> Option<String> {
    let (input, _) = pagination.input();
    let current = args
        .get(input)
        .cloned()
        .or_else(|| declared_default(op, input));
    let document = if pagination.next.starts_with('/') || pagination.more.starts_with('/') {
        op.parse_json_document(response)
    } else {
        None
    };
    if !pagination.more.is_empty()
        && !document
            .as_ref()
            .and_then(|doc| resolve_pointer(doc, &pagination.more))
            .is_some_and(truthy)
    {
        return None;
    }
    let found = if pagination.next == "header:link" {
        let next = link_next(response.header("link"))?;
        from_address(&next, response.url(), input)?
    } else if !pagination.next.is_empty() {
        let value = resolve_pointer(document.as_ref()?, &pagination.next)?;
        match value {
            Value::String(text) if text.starts_with('?') || text.contains("://") => {
                from_address(text, response.url(), input)?
            }
            Value::String(text) => Marker::Value(text.clone()),
            Value::Number(number) => Marker::Value(number.to_string()),
            Value::Bool(true) => Marker::Derived,
            _ => return None,
        }
    } else {
        Marker::Derived
    };
    let value = match found {
        Marker::Value(value) => value,
        Marker::Derived => derived(pagination, current.as_deref(), data)?,
    };
    if !acceptable(pagination, &value) || current.as_deref() == Some(value.as_str()) {
        return None;
    }
    // The marker goes into the next request: it must still be a read of the
    // same site, whatever the site put in it.
    let mut with = args.clone();
    with.insert(input.to_owned(), value.clone());
    if operation_effect(op, &with) != Effect::Read {
        return None;
    }
    let (url, _) = spec.build_request(op, &with).ok()?;
    let url = validate_url(&url).ok()?;
    let destination = validate_url(response.url()).ok()?;
    (url.origin() == destination.origin() && safe_read_url(&url)).then_some(value)
}

enum Marker {
    Value(String),
    Derived,
}

fn declared_default(op: &Operation, input: &str) -> Option<String> {
    op.query
        .get(input)
        .or_else(|| op.body.as_ref().and_then(|b| b.json.get(input)))
        .and_then(|p| p.default.as_ref())
        .map(util::plain)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty() && s != "false",
        _ => true,
    }
}

/// The page after `current`, or the offset past the records returned, when
/// the answer had any.
fn derived(pagination: &Pagination, current: Option<&str>, data: &Value) -> Option<String> {
    let records = records_in(data)?;
    let current: u64 = match current {
        Some(text) => text.parse().ok()?,
        None if pagination.style == "page" => 1,
        None => 0,
    };
    match pagination.style.as_str() {
        "page" => Some((current + 1).to_string()),
        "offset" => Some((current + records as u64).to_string()),
        _ => None,
    }
}

/// How many records an answer holds: the length of the list it is, or of
/// its `items` (what an HTML extraction returns). `None` when it holds
/// none, or is not a list at all.
fn records_in(data: &Value) -> Option<usize> {
    let list = data
        .as_array()
        .or_else(|| data.get("items").and_then(Value::as_array))?;
    (!list.is_empty()).then_some(list.len())
}

/// The `rel="next"` address in a `Link` header.
fn link_next(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let (address, rest) = part.trim().split_once('>')?;
        let address = address.trim().strip_prefix('<')?;
        let next = rest.split(';').map(str::trim).any(|attribute| {
            attribute
                .strip_prefix("rel=")
                .map(|rel| rel.trim_matches('"'))
                .is_some_and(|rel| rel.split_whitespace().any(|r| r == "next"))
        });
        next.then(|| address.to_owned())
    })
}

/// The marker input's value in a next-page address the site gave: a full
/// address of the same origin and path as the request, or a query string
/// (`?page=2&per_page=10`). Only that one value is taken; the address is
/// never fetched as it is.
fn from_address(next: &str, request_url: &str, input: &str) -> Option<Marker> {
    let request = url::Url::parse(request_url).ok()?;
    let next = if next.starts_with('?') {
        request.join(next).ok()?
    } else {
        url::Url::parse(next).ok()?
    };
    if next.origin() != request.origin() || next.path() != request.path() {
        return None;
    }
    next.query_pairs()
        .find(|(key, _)| key == input)
        .map(|(_, value)| Marker::Value(value.into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_link_is_read_from_the_header() {
        let header = r#"<https://api.example/items?page=2&per_page=5>; rel="next", <https://api.example/items?page=9>; rel="last""#;
        assert_eq!(
            link_next(header).as_deref(),
            Some("https://api.example/items?page=2&per_page=5")
        );
        assert_eq!(link_next(r#"<https://api.example/x>; rel="prev""#), None);
        let marker = from_address(
            "https://api.example/items?page=2&per_page=5",
            "https://api.example/items?per_page=5",
            "page",
        );
        assert!(matches!(marker, Some(Marker::Value(v)) if v == "2"));
        assert!(
            from_address(
                "https://other.example/items?page=2",
                "https://api.example/items",
                "page"
            )
            .is_none()
        );
        assert!(matches!(
            from_address("?page=3", "https://api.example/items", "page"),
            Some(Marker::Value(v)) if v == "3"
        ));
    }

    #[test]
    fn markers_that_are_not_markers_are_refused() {
        let cursor = Pagination {
            style: "cursor".into(),
            ..Pagination::default()
        };
        assert!(acceptable(&cursor, "abc123"));
        assert!(!acceptable(&cursor, ""));
        assert!(!acceptable(&cursor, "https://evil.example/x"));
        assert!(!acceptable(&cursor, "-----BEGIN RSA PRIVATE KEY-----"));
        assert!(!acceptable(&cursor, "a\nb"));
        assert!(!acceptable(&cursor, &"x".repeat(MAX_CURSOR + 1)));
        let page = Pagination {
            style: "page".into(),
            ..Pagination::default()
        };
        assert!(acceptable(&page, "2"));
        assert!(!acceptable(&page, "two"));
    }
}
