//! The shape of a page or an answer, without any of its values: which
//! selectors are there and how often, which named fields a page carries,
//! what keys and types the data a page embeds or an answer returns have.
//! An agent reads a shape to pick a selector or a pointer; it never learns
//! what the page or the answer said.
use crate::evidence::selector_word;
use itda_types::util::secret_field;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Selectors listed per page.
pub const STRUCTURE_NODES: usize = 100;
/// How deep a shape goes.
pub const SHAPE_DEPTH: usize = 4;
/// Keys kept per object.
pub const SHAPE_KEYS: usize = 30;
/// Values described in one shape, across all its levels; the data a page
/// embeds shares one budget however many pieces it comes in.
pub const SHAPE_BUDGET: usize = 400;
/// Named fields listed per page.
pub const FIELDS: usize = 100;
/// The most a page's shape may weigh as JSON, in bytes: with what a tool
/// result wraps around it, an agent gets at most 24 KB back.
pub const PAGE_BYTES: usize = 20 * 1024;
/// The name that stands for keys that are ids, hashes or dates: one shape
/// for all of them.
pub const DYNAMIC_KEY: &str = "[dynamic key]";

/// One kind of element on the page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Structure {
    /// `tag#id.class`, built only from names that will still be there
    /// tomorrow.
    pub selector: String,
    /// How many elements the selector matches.
    pub count: usize,
    /// The names of the attributes the first match carries, among those
    /// worth reading (`href`, `src`, `title`, `alt`, `datetime`, `content`,
    /// `name`, `type`, `role`, `aria-label`).
    pub attrs: Vec<String>,
    /// The selector of the first match's parent.
    pub parent: String,
}

/// A named field on the page: `meta[name]`, `input[name]`, or a `data-*`
/// attribute about a user or an account. Its value is never here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldShape {
    /// `meta`, `input` or `data`.
    pub kind: &'static str,
    pub name: String,
    /// Exactly one element on the page has it.
    pub unique: bool,
    /// Its value or content is not empty.
    pub filled: bool,
}

const ATTRS: [&str; 10] = [
    "href",
    "src",
    "title",
    "alt",
    "datetime",
    "content",
    "name",
    "type",
    "role",
    "aria-label",
];

fn selector_of(element: ElementRef<'_>) -> String {
    let value = element.value();
    let mut selector = value.name().to_owned();
    if let Some(id) = value.id().filter(|id| selector_word(id)) {
        selector.push('#');
        selector.push_str(id);
    }
    for class in value.classes().filter(|c| selector_word(c)).take(2) {
        selector.push('.');
        selector.push_str(class);
    }
    selector
}

fn in_head(element: ElementRef<'_>) -> bool {
    element
        .ancestors()
        .filter_map(ElementRef::wrap)
        .any(|a| matches!(a.value().name(), "head" | "script" | "style" | "template"))
}

/// The kinds of element on the page, in document order, at most
/// [`STRUCTURE_NODES`] of them. Scripts, styles and the head are left out.
pub fn structure(html: &str) -> Vec<Structure> {
    let document = Html::parse_document(html);
    let Ok(all) = Selector::parse("body *") else {
        return Vec::new();
    };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut order: Vec<(String, Vec<String>, String)> = Vec::new();
    for element in document.select(&all) {
        if matches!(
            element.value().name(),
            "script" | "style" | "template" | "noscript" | "svg" | "path"
        ) || in_head(element)
        {
            continue;
        }
        let selector = selector_of(element);
        let count = counts.entry(selector.clone()).or_insert(0);
        *count += 1;
        if *count == 1 && order.len() < STRUCTURE_NODES {
            let attrs = ATTRS
                .iter()
                .filter(|name| element.value().attr(name).is_some())
                .map(|name| (*name).to_owned())
                .collect();
            let parent = element
                .parent()
                .and_then(ElementRef::wrap)
                .map(selector_of)
                .unwrap_or_default();
            order.push((selector, attrs, parent));
        }
    }
    order
        .into_iter()
        .map(|(selector, attrs, parent)| Structure {
            count: counts.get(&selector).copied().unwrap_or(0),
            selector,
            attrs,
            parent,
        })
        .collect()
}

/// The named fields on the page, without values: `meta[name]`,
/// `input[name]` that is not a password or a token, and `data-*`
/// attributes whose name says user or account.
pub fn fields(html: &str) -> Vec<FieldShape> {
    let document = Html::parse_document(html);
    let mut out: Vec<FieldShape> = Vec::new();
    let mut seen: BTreeMap<(&'static str, String), (usize, bool)> = BTreeMap::new();
    let mut note = |kind: &'static str, name: &str, filled: bool| {
        let entry = seen.entry((kind, name.to_owned())).or_insert((0, false));
        entry.0 += 1;
        entry.1 |= filled;
    };
    if let Ok(meta) = Selector::parse("meta[name]") {
        for element in document.select(&meta) {
            let name = element.value().attr("name").unwrap_or_default().trim();
            if name.is_empty() || secret_field(name) {
                continue;
            }
            let filled = element
                .value()
                .attr("content")
                .is_some_and(|c| !c.trim().is_empty());
            note("meta", name, filled);
        }
    }
    if let Ok(input) = Selector::parse("input[name], select[name], textarea[name]") {
        for element in document.select(&input) {
            let name = element.value().attr("name").unwrap_or_default().trim();
            let kind = element.value().attr("type").unwrap_or_default();
            if name.is_empty() || secret_field(name) || kind.eq_ignore_ascii_case("password") {
                continue;
            }
            let filled = element
                .value()
                .attr("value")
                .is_some_and(|v| !v.trim().is_empty());
            note("input", name, filled);
        }
    }
    if let Ok(all) = Selector::parse("body *") {
        for element in document.select(&all) {
            for (name, value) in element.value().attrs() {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("data-")
                    && (lower.contains("user") || lower.contains("account"))
                    && !secret_field(&lower)
                {
                    note("data", &lower, !value.trim().is_empty());
                }
            }
        }
    }
    for ((kind, name), (count, filled)) in seen.into_iter().take(FIELDS) {
        out.push(FieldShape {
            kind,
            name,
            unique: count == 1,
            filled,
        });
    }
    out
}

/// Whether a key is an id, a hash, a number, a date, an address, a cache
/// key (`Post:42`) or words: a value used as a name, which changes from one
/// page to the next, so one shape stands for all of them.
pub fn dynamic_key(key: &str) -> bool {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.len() > 48
        || trimmed.contains(['@', '/', ':'])
        || trimmed.chars().any(char::is_whitespace)
    {
        return true;
    }
    let digits = trimmed.chars().filter(|c| c.is_ascii_digit()).count();
    let hex = trimmed.chars().all(|c| c.is_ascii_hexdigit() || c == '-') && trimmed.len() >= 8;
    let numeric = trimmed
        .chars()
        .all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == ':');
    numeric || hex || (digits * 2 > trimmed.len())
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn shape_in(value: &Value, depth: usize, budget: &mut usize) -> Value {
    if *budget == 0 {
        return json!("...");
    }
    *budget -= 1;
    match value {
        Value::Object(map) => {
            if depth == 0 {
                return json!(format!("object[{}]", map.len()));
            }
            let mut out = Map::new();
            let mut dynamic: Option<Value> = None;
            let mut kept = 0;
            for (key, inner) in map {
                if dynamic_key(key) {
                    if dynamic.is_none() {
                        dynamic = Some(shape_in(inner, depth - 1, budget));
                    }
                    continue;
                }
                if kept >= SHAPE_KEYS {
                    out.insert(
                        "...".into(),
                        json!(format!("{} more keys", map.len() - kept)),
                    );
                    break;
                }
                out.insert(key.clone(), shape_in(inner, depth - 1, budget));
                kept += 1;
            }
            if let Some(shape) = dynamic {
                out.insert(DYNAMIC_KEY.into(), shape);
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            if depth == 0 {
                return json!(format!("array[{}]", items.len()));
            }
            // An array does not use up a level: its first element stands
            // for all of them.
            match items.first() {
                Some(first) => json!([shape_in(first, depth, budget)]),
                None => json!([]),
            }
        }
        other => json!(type_name(other)),
    }
}

/// The keys and types of a value, [`SHAPE_DEPTH`] levels deep, at most
/// [`SHAPE_KEYS`] keys per object, with keys that are ids, hashes or dates
/// folded into [`DYNAMIC_KEY`]. Arrays show their first element. Leaves are
/// type names, never values.
pub fn shape(value: &Value) -> Value {
    let mut budget = SHAPE_BUDGET;
    shape_in(value, SHAPE_DEPTH, &mut budget)
}

/// Pointers listed per shape.
pub const POINTERS: usize = 200;

/// One place in a shape: the JSON Pointer of a value (`/items/0/id`) and
/// its type, the way an input or `required_pointers` names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerShape {
    pub pointer: String,
    #[serde(rename = "type")]
    pub kind: String,
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn walk(shape: &Value, at: &str, out: &mut Vec<PointerShape>) {
    if out.len() >= POINTERS {
        return;
    }
    match shape {
        Value::Object(map) => {
            for (key, inner) in map {
                if key == "..." {
                    continue;
                }
                walk(inner, &format!("{at}/{}", escape(key)), out);
            }
        }
        Value::Array(items) => match items.first() {
            Some(first) => walk(first, &format!("{at}/0"), out),
            None => out.push(PointerShape {
                pointer: at.to_owned(),
                kind: "array".into(),
            }),
        },
        Value::String(kind) => out.push(PointerShape {
            pointer: at.to_owned(),
            kind: kind.clone(),
        }),
        _ => {}
    }
}

/// The leaves of a shape made by [`shape`], as pointers with their types,
/// at most [`POINTERS`] of them, in key order.
pub fn pointers(shape: &Value) -> Vec<PointerShape> {
    let mut out = Vec::new();
    walk(shape, "", &mut out);
    out
}

/// The shape of every piece of data the page embeds for its scripts, by
/// the key `extract_ssr_map` gives it, all within one [`SHAPE_BUDGET`].
pub fn ssr_payloads(html: &str) -> BTreeMap<String, Value> {
    let mut budget = SHAPE_BUDGET;
    let mut out = BTreeMap::new();
    for (key, value) in &crate::extract_ssr_map(html) {
        // The key is a script's id or data attribute: kept when it reads as
        // a name a developer chose, folded like any dynamic key otherwise.
        let named = key == "ld+json" || (selector_word(key) && !dynamic_key(key));
        let key = if named {
            key.clone()
        } else {
            DYNAMIC_KEY.to_owned()
        };
        if out.contains_key(&key) {
            continue;
        }
        out.insert(key, shape_in(value, SHAPE_DEPTH, &mut budget));
    }
    out
}

/// Everything above for one page, as the knowledge base stores it, within
/// [`PAGE_BYTES`].
pub fn page(html: &str) -> Value {
    let mut page = json!({
        "structure": structure(html),
        "fields": fields(html),
        "ssr_payloads": ssr_payloads(html),
    });
    fit(&mut page, PAGE_BYTES);
    page
}

fn weight(value: &Value) -> usize {
    serde_json::to_string(value).map_or(0, |text| text.len())
}

/// Make a page's shape weigh at most `max` bytes as JSON: rows go from the
/// end of `structure` or `fields`, whichever is longer, then the embedded
/// data. Says whether anything went; a shape cut down carries
/// `"truncated": true`.
pub fn fit(page: &mut Value, max: usize) -> bool {
    if weight(page) <= max {
        return false;
    }
    if let Some(map) = page.as_object_mut() {
        map.insert("truncated".into(), true.into());
    }
    let rows =
        |page: &Value, key: &str| page.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    while weight(page) > max {
        let key = if rows(page, "structure") >= rows(page, "fields") {
            "structure"
        } else {
            "fields"
        };
        match page.get_mut(key).and_then(Value::as_array_mut) {
            Some(list) if !list.is_empty() => {
                list.pop();
            }
            _ => break,
        }
    }
    if weight(page) > max
        && let Some(payloads) = page.get_mut("ssr_payloads")
    {
        *payloads = json!({});
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><head><title>Board</title><meta name="description" content="A board"><meta name="csrf-token" content="SECRET-TOKEN-1"><script>var x=1</script></head>
<body><div id="app" class="shell css-1x2y3z4"><nav class="menu"><a href="/">Home</a></nav>
<table class="notices"><tr class="row"><td><a href="/n/1" title="t">Secret Title One</a></td><td><time datetime="2026-01-01">Jan 1</time></td></tr>
<tr class="row"><td><a href="/n/2">Secret Title Two</a></td><td>Jan 2</td></tr></table>
<form action="/login" method="post"><input name="user" value="alice"><input name="password" type="password" value="hunter2"><input name="session_token" value="tok-999"><input name="q"></form>
<span data-user-id="42" data-account-name="alice">hi</span>
<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"items":[{"id":1,"title":"Secret Title One"}],"session":"abc"},"byId":{"12345678":{"n":1},"87654321":{"n":2}}},"buildId":"zzz"}</script>
</body></html>"#;

    fn text(value: &Value) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }

    #[test]
    fn structure_lists_selectors_counts_and_attribute_names_only() {
        let rows = structure(PAGE);
        let row = rows.iter().find(|r| r.selector == "tr.row").unwrap();
        assert_eq!(row.count, 2);
        assert_eq!(row.parent, "tbody");
        let link = rows.iter().find(|r| r.selector == "a").unwrap();
        assert_eq!((link.count, link.parent.as_str()), (3, "nav.menu"));
        assert_eq!(link.attrs, ["href"]);
        let time = rows.iter().find(|r| r.selector == "time").unwrap();
        assert_eq!(time.attrs, ["datetime"]);
        assert!(rows.iter().all(|r| r.selector != "script"));
        // Build hashes never make it into a selector.
        assert!(rows.iter().any(|r| r.selector == "div#app.shell"));
        let all = text(&json!(rows));
        for value in ["Secret Title", "alice", "hunter2", "SECRET-TOKEN", "/n/1"] {
            assert!(!all.contains(value), "{value} in {all}");
        }
    }

    #[test]
    fn fields_name_what_a_page_carries_without_values_or_secrets() {
        let rows = fields(PAGE);
        let names: Vec<(&str, &str, bool, bool)> = rows
            .iter()
            .map(|r| (r.kind, r.name.as_str(), r.unique, r.filled))
            .collect();
        assert_eq!(
            names,
            [
                ("data", "data-account-name", true, true),
                ("data", "data-user-id", true, true),
                ("input", "q", true, false),
                ("input", "user", true, true),
                ("meta", "description", true, true),
            ]
        );
        let all = text(&json!(rows));
        for value in [
            "alice",
            "hunter2",
            "tok-999",
            "42",
            "SECRET",
            "password",
            "csrf",
            "session_token",
        ] {
            assert!(!all.contains(value), "{value} in {all}");
        }
    }

    #[test]
    fn a_shape_has_keys_and_types_folds_dynamic_keys_and_stays_bounded() {
        let payloads = ssr_payloads(PAGE);
        let next = &payloads["__NEXT_DATA__"];
        assert_eq!(next["props"]["pageProps"]["items"][0]["title"], "string");
        assert_eq!(next["props"]["pageProps"]["items"][0]["id"], "integer");
        assert_eq!(next["props"]["pageProps"]["session"], "string");
        assert_eq!(next["buildId"], "string");
        assert_eq!(next["props"]["byId"][DYNAMIC_KEY]["n"], "integer");
        assert!(next["props"]["byId"].get("12345678").is_none());
        let all = text(&json!(payloads));
        let named = ssr_payloads(
            r#"<script id="profile-4f0a4207d0" type="application/json">{"a":1}</script><script id="alice@example.org" type="application/json">{"b":1}</script>"#,
        );
        assert_eq!(named.keys().collect::<Vec<_>>(), [DYNAMIC_KEY]);
        for value in ["Secret", "abc", "zzz", "12345678"] {
            assert!(!all.contains(value), "{value} in {all}");
        }
        let mut wide = Map::new();
        for i in 0..50 {
            wide.insert(
                format!("key{i}"),
                json!({"deep": {"deeper": {"deepest": {"beyond": 1}}}}),
            );
        }
        let shaped = shape(&Value::Object(wide));
        assert_eq!(shaped.as_object().unwrap().len(), SHAPE_KEYS + 1);
        assert_eq!(shaped["key0"]["deep"]["deeper"]["deepest"], "object[1]");
        assert!(shaped["..."].as_str().unwrap().contains("more keys"));
        let mut huge = Map::new();
        for i in 0..SHAPE_KEYS {
            let mut inner = Map::new();
            for j in 0..SHAPE_KEYS {
                inner.insert(format!("k{i}_{j}"), json!({"a": {"b": 1}}));
            }
            huge.insert(format!("top{i}"), Value::Object(inner));
        }
        assert!(text(&shape(&Value::Object(huge))).len() < 24 * 1024);
        assert_eq!(shape(&json!([])), json!([]));
        assert_eq!(shape(&json!([1.5, 2])), json!(["number"]));
        assert_eq!(shape(&json!(null)), "null");
    }

    #[test]
    fn pointers_name_every_leaf_with_its_type() {
        let shaped = shape(&json!({"items": [{"id": 1, "a~b": "x"}], "total": 2, "none": []}));
        let rows: Vec<(String, String)> = pointers(&shaped)
            .into_iter()
            .map(|p| (p.pointer, p.kind))
            .collect();
        assert_eq!(
            rows,
            [
                ("/items/0/a~0b".to_owned(), "string".to_owned()),
                ("/items/0/id".to_owned(), "integer".to_owned()),
                ("/none".to_owned(), "array".to_owned()),
                ("/total".to_owned(), "integer".to_owned()),
            ]
        );
    }

    #[test]
    fn a_page_shape_fits_in_one_tool_result() {
        let mut html = String::from("<html><body>");
        let long = "abcdefghij".repeat(25);
        for i in 0..400 {
            html.push_str(&format!(
                "<div class=\"block{i}\"><input name=\"field{i}{long}\"></div>"
            ));
        }
        html.push_str("<script id=\"__NEXT_DATA__\" type=\"application/json\">{");
        for i in 0..300 {
            html.push_str(&format!("\"key{i}\":{{\"a\":{{\"b\":1}}}},"));
        }
        html.push_str("\"end\":1}</script></body></html>");
        let shaped = page(&html);
        assert!(text(&shaped).len() <= PAGE_BYTES, "{}", text(&shaped).len());
        assert_eq!(shaped["truncated"], true);
        assert!(!shaped["structure"].as_array().unwrap().is_empty());
        assert!(!shaped["fields"].as_array().unwrap().is_empty());
        let small = page(PAGE);
        assert!(small.get("truncated").is_none());
    }

    #[test]
    fn dynamic_keys_are_ids_hashes_and_dates() {
        for key in [
            "12345678",
            "2026-01-01",
            "4f0a4207d04f4a0fb4e059d26bd54b6d",
            "a1b2c3d4e5",
            "12:30",
            "alice@example.com",
            "Post:42",
            "/n/1",
            "A record title",
        ] {
            assert!(dynamic_key(key), "{key}");
        }
        for key in ["id", "title", "pageProps", "h1", "utf8", "sha256", ""] {
            assert!(!dynamic_key(key), "{key}");
        }
    }
}
