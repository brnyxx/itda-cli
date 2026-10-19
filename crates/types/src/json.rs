//! JSON helpers shared by extraction, verification and credential recipes.
//!
//! Derived from Hycli `src/html.rs` (`resolve_pointer`), Apache-2.0. Moved here
//! because credential and response checks use it without parsing HTML.
use serde_json::Value;

/// Resolve a JSON Pointer (RFC 6901), with fallback matching for object keys
/// that contain an unescaped `/`.
pub fn resolve_pointer<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        return Some(value);
    }
    if let Some(found) = value.pointer(pointer) {
        return Some(found);
    }
    let rest = pointer.strip_prefix('/')?;
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if rest == k {
                    return Some(v);
                }
                if let Some(after) = rest
                    .strip_prefix(k.as_str())
                    .and_then(|s| s.strip_prefix('/'))
                    && let Some(found) = resolve_pointer(v, &format!("/{after}"))
                {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => {
            let (idx_str, after) = rest.split_once('/').unwrap_or((rest, ""));
            let idx: usize = idx_str.parse().ok()?;
            let elem = arr.get(idx)?;
            if after.is_empty() {
                Some(elem)
            } else {
                resolve_pointer(elem, &format!("/{after}"))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pointers_resolve_standard_and_slash_containing_keys() {
        let value = json!({"a":{"b":[{"c":1}]},"routes/x.y":{"items":[{"id":"7"}]}});
        assert_eq!(resolve_pointer(&value, ""), Some(&value));
        assert_eq!(resolve_pointer(&value, "/a/b/0/c"), Some(&json!(1)));
        assert_eq!(
            resolve_pointer(&value, "/routes/x.y/items/0/id"),
            Some(&json!("7"))
        );
        assert_eq!(resolve_pointer(&value, "/a/missing"), None);
        assert_eq!(resolve_pointer(&value, "no-leading-slash"), None);
    }
}
