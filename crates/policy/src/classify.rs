//! Read/write classification. An operation is a read only when its method, name,
//! path, inputs and GraphQL document all agree; anything ambiguous needs review.
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
        if ["action", "operation", "op", "method", "command", "mode"]
            .contains(&key.to_ascii_lowercase().as_str())
            && dangerous_intent(value)
        {
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
        if let Some(query) = resolved.get("query") {
            use graphql_parser::query::{Definition, OperationDefinition};
            // A document nested past the limit is never parsed: it needs
            // review, not a stack.
            if itda_spec::graphql_too_deep(query) {
                return Effect::Unknown;
            }
            match graphql_parser::parse_query::<&str>(query) {
                Ok(document) => {
                    let operations: Vec<_> = document
                        .definitions
                        .iter()
                        .filter_map(|definition| {
                            if let Definition::Operation(operation) = definition {
                                Some(operation)
                            } else {
                                None
                            }
                        })
                        .collect();
                    if operations.iter().any(|operation| {
                        matches!(
                            operation,
                            OperationDefinition::Mutation(_) | OperationDefinition::Subscription(_)
                        )
                    }) {
                        return Effect::Write;
                    }
                    if operations.len() == 1 && op.effect == Effect::Read && !op.evidence.is_empty()
                    {
                        return Effect::Read;
                    }
                    return Effect::Unknown;
                }
                Err(_)
                    if op.path.to_ascii_lowercase().contains("graphql")
                        || query.trim_start().starts_with(['{', '#'])
                        || ["query", "mutation", "subscription", "fragment"]
                            .iter()
                            .any(|word| query.trim_start().starts_with(word)) =>
                {
                    return Effect::Unknown;
                }
                Err(_) => {} // A plain search string is not a GraphQL document.
            }
        } else if op.path.to_ascii_lowercase().contains("graphql") {
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
            (["action", "operation", "op", "method", "command", "mode"]
                .contains(&key.to_ascii_lowercase().as_str())
                && value.as_str().is_some_and(dangerous_intent))
                || nested_change_intent(value)
        }),
        serde_json::Value::Array(array) => array.iter().any(nested_change_intent),
        _ => false,
    }
}

/// The same semantic check is used for observation URLs and every redirect.
pub fn safe_read_url(url: &url::Url) -> bool {
    !dangerous_intent(url.path())
        && !url.query_pairs().any(|(key, value)| {
            util::secret_field(&key)
                || dangerous_intent(&key)
                || (["action", "operation", "op", "command", "method", "mode"]
                    .contains(&key.to_ascii_lowercase().as_str())
                    && dangerous_intent(&value))
        })
}
