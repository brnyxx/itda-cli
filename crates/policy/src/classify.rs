//! Read/write classification. An operation is a read only when its method, name,
//! path, inputs and GraphQL document all agree; anything ambiguous needs review.
//! The request itda is about to send is read once more after its template is
//! rendered ([`rendered_request_effect`]), so a literal in a body template or
//! an address a site handed out cannot pass as a read on declared inputs alone.
//!
//! Derived from Hycli `src/policy.rs` (`dangerous_intent`, `operation_effect`,
//! `safe_read_url`), Apache-2.0. Logic unchanged; the regex is compiled once.
use itda_spec::{Effect, Operation};
use itda_types::util;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// An operation's semantics matter even when its transport says GET.
pub fn dangerous_intent(value: &str) -> bool {
    let decoded =
        url::form_urlencoded::parse(format!("x={}", value.replace('+', "%2B")).as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_else(|| value.into());
    static CAMEL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"([a-z0-9])([A-Z])").expect("constant"));
    let camel = CAMEL.replace_all(&decoded, "$1-$2");
    let lower = camel.to_ascii_lowercase();
    let words: Vec<_> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    const CHANGES: &[&str] = &[
        "add",
        "edit",
        "change",
        "toggle",
        "commit",
        "upsert",
        "redeem",
        "confirm",
        "authorize",
        "login",
        "signin",
        "logout",
        "logoff",
        "signout",
        "delete",
        "remove",
        "destroy",
        "erase",
        "purge",
        "drop",
        "create",
        "insert",
        "update",
        "patch",
        "submit",
        "send",
        "publish",
        "unpublish",
        "upload",
        "transfer",
        "purchase",
        "checkout",
        "pay",
        "vote",
        "like",
        "unlike",
        "follow",
        "unfollow",
        "subscribe",
        "unsubscribe",
        "revoke",
        "reset",
        "activate",
        "deactivate",
        "enable",
        "disable",
        "execute",
        "trigger",
        "accept",
        "reject",
        "approve",
        "invite",
        "cancel",
        "terminate",
        "stop",
        "restart",
        "save",
        "markread",
        "markseen",
        "increment",
    ];
    words.iter().any(|w| CHANGES.contains(w))
        || ["sign-out", "log-out", "mark-as-read", "mark-as-seen"]
            .iter()
            .any(|w| lower.contains(w))
}
/// Inputs whose value says what the request does: `?action=delete`.
const INTENT_KEYS: [&str; 6] = ["action", "operation", "op", "method", "command", "mode"];

pub fn operation_effect(op: &Operation, args: &BTreeMap<String, String>) -> Effect {
    let method = op.method.to_ascii_uppercase();
    if dangerous_intent(&op.path) || dangerous_intent(&op.name) {
        return Effect::Write;
    }
    if ["PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
        return Effect::Write;
    }
    if op.effect == Effect::Write {
        return Effect::Write;
    }
    let mut resolved = BTreeMap::new();
    for (key, param) in op
        .params
        .iter()
        .chain(op.query.iter())
        .chain(op.body.iter().flat_map(|b| b.inputs()))
    {
        if let Some(value) = &param.default {
            resolved.insert(key.clone(), util::plain(value));
        }
    }
    resolved.extend(args.clone());
    for (key, value) in &resolved {
        if INTENT_KEYS.contains(&key.to_ascii_lowercase().as_str()) && dangerous_intent(value) {
            return Effect::Write;
        }
    }
    for key in op.params.keys() {
        if resolved
            .get(key)
            .is_some_and(|value| dangerous_intent(value))
        {
            return Effect::Write;
        }
    }
    for value in resolved.values() {
        if serde_json::from_str::<serde_json::Value>(value)
            .ok()
            .is_some_and(|value| nested_change_intent(&value))
        {
            return Effect::Write;
        }
    }
    // Parse every candidate GraphQL document, regardless of the endpoint's name.
    // Conservatively require review for any mutation, subscription or ambiguous document.
    if method == "POST" || method == "GET" {
        let graphql_path = op.path.to_ascii_lowercase().contains("graphql");
        if let Some(query) = resolved.get("query") {
            match graphql_document(query, graphql_path) {
                Some(Document::Write) => return Effect::Write,
                Some(Document::Read) if op.effect == Effect::Read && !op.evidence.is_empty() => {
                    return Effect::Read;
                }
                Some(_) => return Effect::Unknown,
                None => {} // A plain search string is not a GraphQL document.
            }
        } else if graphql_path {
            return Effect::Unknown;
        }
    }
    if method == "GET" || method == "HEAD" || method == "OPTIONS" {
        return if op.effect == Effect::Read && !op.evidence.is_empty() {
            Effect::Read
        } else {
            Effect::Unknown
        };
    }
    if method == "POST" && op.effect == Effect::Read && !op.evidence.is_empty() {
        // Explicitly documented search/query/lookup operations may use POST.
        let lower = format!("{} {}", op.name, op.path).to_ascii_lowercase();
        if [
            "search", "query", "lookup", "list", "find", "count", "preview", "resolve", "get",
            "fetch", "read", "filter", "check", "inspect",
        ]
        .iter()
        .any(|word| {
            lower
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|p| p == *word)
        }) {
            return Effect::Read;
        }
    }
    Effect::Unknown
}

fn nested_change_intent(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            (INTENT_KEYS.contains(&key.to_ascii_lowercase().as_str())
                && value.as_str().is_some_and(dangerous_intent))
                || nested_change_intent(value)
        }),
        serde_json::Value::Array(array) => array.iter().any(nested_change_intent),
        _ => false,
    }
}

/// What a GraphQL document found in a request says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Document {
    /// One query operation.
    Read,
    /// A mutation or a subscription.
    Write,
    /// A document itda cannot read: nested too deep, broken, or several
    /// operations at once.
    Unknown,
}

/// Read `text` as a GraphQL document. `None` when it is a plain string
/// (a search term) rather than a document; on a GraphQL path every value of
/// a document input is a document.
fn graphql_document(text: &str, graphql_path: bool) -> Option<Document> {
    use graphql_parser::query::{Definition, OperationDefinition};
    // A document nested past the limit is never parsed: it needs review,
    // not a stack.
    if itda_spec::graphql_too_deep(text) {
        return Some(Document::Unknown);
    }
    match graphql_parser::parse_query::<&str>(text) {
        Ok(document) => {
            let operations: Vec<_> = document
                .definitions
                .iter()
                .filter_map(|definition| match definition {
                    Definition::Operation(operation) => Some(operation),
                    Definition::Fragment(_) => None,
                })
                .collect();
            if operations.iter().any(|operation| {
                matches!(
                    operation,
                    OperationDefinition::Mutation(_) | OperationDefinition::Subscription(_)
                )
            }) {
                return Some(Document::Write);
            }
            Some(if operations.len() == 1 {
                Document::Read
            } else {
                Document::Unknown
            })
        }
        Err(_)
            if graphql_path
                || text.trim_start().starts_with(['{', '#'])
                || ["query", "mutation", "subscription", "fragment"]
                    .iter()
                    .any(|word| text.trim_start().starts_with(word)) =>
        {
            Some(Document::Unknown)
        }
        Err(_) => None,
    }
}

/// A query or body field that carries a GraphQL document: `query` anywhere,
/// and `q` or `document` on a GraphQL path.
fn document_key(key: &str, graphql_path: bool) -> bool {
    key == "query" || (graphql_path && (key == "q" || key == "document"))
}

/// The same semantic check is used for observation URLs and every redirect
/// and page address a site hands out. A GraphQL document in the address is
/// read too: a mutation or subscription is not a read, whatever the path.
pub fn safe_read_url(url: &url::Url) -> bool {
    let graphql_path = url.path().to_ascii_lowercase().contains("graphql");
    !dangerous_intent(url.path())
        && !url.query_pairs().any(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            util::secret_field(&key)
                || dangerous_intent(&key)
                || (INTENT_KEYS.contains(&lower.as_str()) && dangerous_intent(&value))
                || (document_key(&lower, graphql_path)
                    && matches!(
                        graphql_document(&value, graphql_path),
                        Some(Document::Write | Document::Unknown)
                    ))
        })
}

/// What the request itda is about to send says it does, with the reason in
/// plain words when it is not a read. `reason` is empty for a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rendered {
    pub effect: Effect,
    pub reason: &'static str,
}

impl Rendered {
    const READ: Self = Self {
        effect: Effect::Read,
        reason: "",
    };
    fn write(reason: &'static str) -> Self {
        Self {
            effect: Effect::Write,
            reason,
        }
    }
    fn unknown(reason: &'static str) -> Self {
        Self {
            effect: Effect::Unknown,
            reason,
        }
    }
}

/// What a walk over a rendered request found so far.
#[derive(Default)]
struct Found {
    /// A GraphQL query document was found in the address or the body.
    document: bool,
    /// The first reason the request cannot be classified, if any.
    unknown: Option<&'static str>,
}

const UNREADABLE_DOCUMENT: &str = "the request carries a GraphQL document itda cannot read";

/// Classify the request as it will be sent: the rendered address and body,
/// not the declared inputs. Every query key, form field and JSON key goes
/// through [`dangerous_intent`]; a value does when its key says what the
/// request does (`action`, `mode`, ...); a GraphQL document found in `query`
/// (or `q`/`document` on a GraphQL path) is parsed for its operation type.
/// A GraphQL path with no document is unknown. `form` says the body is
/// form-encoded rather than JSON.
pub fn rendered_request_effect(url: &url::Url, body: Option<&[u8]>, form: bool) -> Rendered {
    let graphql_path = url.path().to_ascii_lowercase().contains("graphql");
    if dangerous_intent(url.path()) {
        return Rendered::write("address names a change");
    }
    let mut found = Found::default();
    if let Some(verdict) = classify_pairs(url.query_pairs(), "address", graphql_path, &mut found) {
        return verdict;
    }
    if let Some(body) = body {
        let verdict = if form {
            classify_pairs(
                url::form_urlencoded::parse(body),
                "body",
                graphql_path,
                &mut found,
            )
        } else {
            match serde_json::from_slice::<serde_json::Value>(body) {
                Ok(value) => classify_value(&value, "body", 0, graphql_path, &mut found),
                Err(_) => Some(Rendered::unknown(
                    "the body is not the JSON its definition declares",
                )),
            }
        };
        if let Some(verdict) = verdict {
            return verdict;
        }
    }
    if let Some(reason) = found.unknown {
        return Rendered::unknown(reason);
    }
    if graphql_path && !found.document {
        return Rendered::unknown("the body names no GraphQL document");
    }
    Rendered::READ
}

/// Query pairs or form fields. `Some` ends the walk with a change.
fn classify_pairs<'a>(
    pairs: impl Iterator<Item = (std::borrow::Cow<'a, str>, std::borrow::Cow<'a, str>)>,
    place: &'static str,
    graphql_path: bool,
    found: &mut Found,
) -> Option<Rendered> {
    for (key, value) in pairs {
        if let Some(verdict) = classify_field(&key, &value, place, graphql_path, found) {
            return Some(verdict);
        }
        // A field whose value is JSON is walked like a JSON body, as the
        // declared inputs are.
        if value.trim_start().starts_with(['{', '['])
            && let Ok(nested) = serde_json::from_str::<serde_json::Value>(&value)
            && let Some(verdict) = classify_value(&nested, place, 1, graphql_path, found)
        {
            return Some(verdict);
        }
    }
    None
}

/// One key and string value, wherever it sits.
fn classify_field(
    key: &str,
    value: &str,
    place: &'static str,
    graphql_path: bool,
    found: &mut Found,
) -> Option<Rendered> {
    let lower = key.to_ascii_lowercase();
    if dangerous_intent(key) || (INTENT_KEYS.contains(&lower.as_str()) && dangerous_intent(value)) {
        return Some(Rendered::write(if place == "address" {
            "address asks for a change"
        } else {
            "body asks for a change"
        }));
    }
    if document_key(&lower, graphql_path) {
        match graphql_document(value, graphql_path) {
            Some(Document::Write) => {
                return Some(Rendered::write(if place == "address" {
                    "address names a mutation"
                } else {
                    "body names a mutation"
                }));
            }
            Some(Document::Read) => found.document = true,
            Some(Document::Unknown) => {
                found.unknown.get_or_insert(UNREADABLE_DOCUMENT);
            }
            None => {}
        }
    }
    None
}

/// A JSON value at `depth`, bounded like a GraphQL document is.
fn classify_value(
    value: &serde_json::Value,
    place: &'static str,
    depth: usize,
    graphql_path: bool,
    found: &mut Found,
) -> Option<Rendered> {
    if depth > itda_spec::GRAPHQL_MAX_DEPTH {
        return Some(Rendered::unknown("the body nests deeper than 32 levels"));
    }
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                let text = value.as_str().unwrap_or_default();
                if let Some(verdict) = classify_field(key, text, place, graphql_path, found) {
                    return Some(verdict);
                }
                if let Some(verdict) = classify_value(value, place, depth + 1, graphql_path, found)
                {
                    return Some(verdict);
                }
            }
            None
        }
        serde_json::Value::Array(array) => array
            .iter()
            .find_map(|value| classify_value(value, place, depth + 1, graphql_path, found)),
        _ => None,
    }
}
