//! Bounded structural HTML evidence and declarative extraction, without script execution.
//!
//! Derived from Hycli `src/html.rs` (Apache-2.0). Changes: only the parts that
//! execute a declared extraction or verify a response were kept (`Field`,
//! `Extraction`, embedded SSR JSON). Evidence collection for site preparation
//! (`structure`, `fields`, `forms`, `infer_html_extraction`, `nav_list_routes`)
//! will live with the preparation layer. A site-specific rule for one
//! marketplace's Remix payload was removed. `resolve_pointer` moved to
//! `itda_types::json`.
pub mod evidence;
pub mod shape;

use itda_types::json::resolve_pointer;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use scraper::Html as Document;

/// True when `selector` is a CSS selector this crate can evaluate.
pub fn selector_valid(selector: &str) -> bool {
    selector.len() <= 500 && Selector::parse(selector).is_ok()
}

/// Parse a full HTML document for [`Field::extract`] and [`Field::is_present`].
/// The page's `<title>`, trimmed; empty when it has none.
pub fn title(html: &str) -> String {
    let document = Html::parse_document(html);
    Selector::parse("title")
        .ok()
        .and_then(|selector| document.select(&selector).next())
        .map(|element| element.text().collect::<String>().trim().to_owned())
        .unwrap_or_default()
}

pub fn parse_document(html: &str) -> Html {
    Html::parse_document(html)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Field {
    pub selector: String,
    /// Empty selects text; otherwise selects one named attribute.
    pub attribute: String,
}

impl Field {
    pub fn extract(&self, document: &Html) -> Option<String> {
        self.extract_bounded(document, 320)
    }
    /// Response guards check structure, not the size limit of an account identity.
    pub fn is_present(&self, document: &Html) -> bool {
        self.extract_bounded(document, usize::MAX).is_some()
    }
    fn extract_bounded(&self, document: &Html, max_length: usize) -> Option<String> {
        if self.selector.is_empty() || self.selector.len() > 500 || self.attribute.len() > 80 {
            return None;
        }
        let selector = Selector::parse(&self.selector).ok()?;
        let mut matches = document.select(&selector);
        let element = matches.next()?;
        // Never silently choose a person from a list of profiles or an account switcher.
        if matches.next().is_some() {
            return None;
        }
        let value = if self.attribute.is_empty() {
            element_text(element)
        } else {
            element.value().attr(&self.attribute)?.to_string()
        };
        let value = value.trim();
        if value.is_empty() || value.len() > max_length || value.contains(['\r', '\n']) {
            None
        } else {
            Some(value.into())
        }
    }
}

/// Repeated page records and optional page-level fields, declared from observed DOM evidence.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Extraction {
    pub items: String,
    pub fields: std::collections::BTreeMap<String, OutputField>,
    pub page: std::collections::BTreeMap<String, OutputField>,
    /// A hard output bound, never an instruction to fetch more pages.
    pub limit: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputField {
    /// Empty selects the record itself; nonempty selectors are relative to the record.
    pub selector: String,
    pub attribute: String,
    pub required: bool,
    pub absolute_url: bool,
}
impl Extraction {
    pub fn valid(&self) -> bool {
        (!self.fields.is_empty() || !self.page.is_empty())
            && self.fields.len() + self.page.len() <= 40
            && self.limit <= 1000
            && self.items.len() <= 500
            && (self.items.is_empty() || Selector::parse(&self.items).is_ok())
            && self
                .fields
                .iter()
                .chain(self.page.iter())
                .all(|(name, field)| {
                    stable_name(name)
                        && !itda_types::util::secret_field(name)
                        && field.selector.len() <= 500
                        && field.attribute.len() <= 80
                        && (field.selector.is_empty() || Selector::parse(&field.selector).is_ok())
                        && !itda_types::util::secret_field(&field.attribute)
                })
    }
    pub fn extract(&self, html: &str, base: &str) -> Option<Value> {
        if !self.valid() {
            return None;
        }
        let document = Html::parse_document(html);
        let base = url::Url::parse(base).ok()?;
        let mut output = extract_fields(document.root_element(), &self.page, &base)?;
        if !self.fields.is_empty() {
            let records = if self.items.is_empty() {
                vec![document.root_element()]
            } else {
                let matched: Vec<_> = document
                    .select(&Selector::parse(&self.items).ok()?)
                    .take(if self.limit == 0 { 100 } else { self.limit })
                    .collect();
                if matched.is_empty() {
                    return None;
                }
                matched
            };
            let items: Option<Vec<_>> = records
                .into_iter()
                .map(|record| extract_fields(record, &self.fields, &base).map(Value::Object))
                .collect();
            output.insert("items".into(), Value::Array(items?));
        }
        Some(Value::Object(output))
    }
}
fn extract_fields(
    root: scraper::ElementRef<'_>,
    fields: &std::collections::BTreeMap<String, OutputField>,
    base: &url::Url,
) -> Option<serde_json::Map<String, Value>> {
    let mut output = serde_json::Map::new();
    for (name, field) in fields {
        let element = if field.selector.is_empty() {
            Some(root)
        } else {
            let selector = Selector::parse(&field.selector).ok()?;
            let mut matches = root.select(&selector);
            let first = matches.next();
            if matches.next().is_some() {
                None
            } else {
                first
            }
        };
        let value = element
            .and_then(|element| {
                if field.attribute.is_empty() {
                    Some(element_text(element))
                } else {
                    element.value().attr(&field.attribute).map(str::to_owned)
                }
            })
            .filter(|value| !value.is_empty() && value.len() <= 16_384);
        let value = if field.absolute_url {
            value
                .and_then(|value| base.join(&value).ok())
                .filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.username().is_empty()
                        && url.password().is_none()
                })
                .map(|url| url.to_string())
        } else {
            value
        };
        if field.required && value.is_none() {
            return None;
        }
        output.insert(
            name.clone(),
            value.map(Value::String).unwrap_or(Value::Null),
        );
    }
    Some(output)
}

pub(crate) fn stable_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

pub(crate) fn element_text(element: scraper::ElementRef<'_>) -> String {
    element
        .descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            let hidden = node
                .ancestors()
                .filter_map(scraper::ElementRef::wrap)
                .any(|parent| {
                    matches!(
                        parent.value().name(),
                        "script" | "style" | "noscript" | "template"
                    ) || parent.value().attr("hidden").is_some()
                        || parent.value().attr("aria-hidden") == Some("true")
                });
            (!hidden).then_some(text.as_ref())
        })
        .collect::<Vec<&str>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_next_f_props(val: &Value, out: &mut serde_json::Map<String, Value>, depth: usize) {
    if depth > 24 {
        return;
    }
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                if matches!(
                    k.as_str(),
                    "data"
                        | "posts"
                        | "items"
                        | "articles"
                        | "results"
                        | "initialData"
                        | "nodes"
                        | "edges"
                        | "list"
                        | "feed"
                        | "entries"
                        | "trending"
                        | "searchParams"
                        | "params"
                        | "buildId"
                ) {
                    let non_empty = match v {
                        Value::Null => false,
                        Value::Array(a) => !a.is_empty(),
                        Value::Object(o) => !o.is_empty(),
                        Value::String(s) => !s.is_empty(),
                        _ => true,
                    };
                    if non_empty {
                        out.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                } else if !matches!(
                    k.as_str(),
                    "children" | "fallback" | "parallelRoutes" | "styles" | "tree"
                ) && v
                    .as_array()
                    .is_some_and(|arr| !arr.is_empty() && arr.first().is_some_and(Value::is_object))
                {
                    out.entry(k.clone()).or_insert_with(|| v.clone());
                }
                collect_next_f_props(v, out, depth + 1);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_next_f_props(item, out, depth + 1);
            }
        }
        _ => {}
    }
}

pub fn extract_ssr_map(html: &str) -> std::collections::BTreeMap<String, Value> {
    let document = Html::parse_document(html);
    let mut payloads = std::collections::BTreeMap::new();
    let mut next_f_stream = String::new();
    let window_assign_re = regex::Regex::new(r"window\.([A-Za-z0-9_]+)\s*=\s*").expect("constant");

    if let Ok(sel) = Selector::parse("script") {
        for script in document.select(&sel).take(250) {
            let id = script.value().id().unwrap_or("");
            let script_key = if !id.is_empty() {
                id
            } else {
                script
                    .value()
                    .attr("data-target")
                    .or_else(|| script.value().attr("data-island"))
                    .or_else(|| script.value().attr("data-ssr"))
                    .unwrap_or("")
            };
            let r#type = script.value().attr("type").unwrap_or("");
            let raw: String = script.text().collect();
            let raw = raw.trim();
            if raw.is_empty() || raw.len() > 4 * 1024 * 1024 {
                continue;
            }
            if (!script_key.is_empty()
                && (script_key == "__NEXT_DATA__"
                    || script_key == "__NUXT_DATA__"
                    || r#type.contains("json")
                    || script_key.ends_with(".embeddedData")))
                || r#type.eq_ignore_ascii_case("application/ld+json")
            {
                if let Ok(val) = serde_json::from_str::<Value>(raw) {
                    if !script_key.is_empty() {
                        payloads.insert(script_key.to_owned(), val);
                    } else if let Some(existing) = payloads.get_mut("ld+json") {
                        if let (Value::Object(ex_map), Value::Object(new_map)) = (existing, &val) {
                            for (k, v) in new_map {
                                ex_map.entry(k.clone()).or_insert_with(|| v.clone());
                            }
                            let blocks = ex_map
                                .entry("blocks".to_owned())
                                .or_insert_with(|| Value::Array(vec![]));
                            if let Value::Array(arr) = blocks {
                                arr.push(val);
                            }
                        }
                    } else {
                        payloads.insert("ld+json".to_owned(), val);
                    }
                }
                continue;
            }
            for cap in window_assign_re.captures_iter(raw).take(20) {
                let ident = &cap[1];
                if ident != "INIT_DATA" && !ident.starts_with("__") {
                    continue;
                }
                let Some(m) = cap.get(0) else {
                    continue;
                };
                let rhs = raw[m.end()..].trim_start();
                if !rhs.starts_with('{') && !rhs.starts_with('[') {
                    continue;
                }
                let mut stream = serde_json::Deserializer::from_str(rhs).into_iter::<Value>();
                if let Some(Ok(mut val)) = stream.next() {
                    if ident == "__remixContext" {
                        if let Some(Value::Object(loader_map)) =
                            val.pointer("/state/loaderData").cloned()
                        {
                            let mut route_data = serde_json::Map::new();
                            for (rk, rv) in loader_map {
                                if rk != "root"
                                    && let Value::Object(rmap) = rv
                                {
                                    for (fk, fv) in rmap {
                                        route_data.entry(fk).or_insert(fv);
                                    }
                                }
                            }
                            if !route_data.is_empty()
                                && let Value::Object(top) = &mut val
                            {
                                top.insert("routeData".into(), Value::Object(route_data));
                            }
                        }
                    } else if ident == "__APOLLO_STATE__"
                        && let Value::Object(top) = &mut val
                    {
                        // The cache is a flat map keyed `Typename:id`. Records
                        // with a title stay under `items`, as before, and every
                        // record is also grouped under its typename, so a
                        // definition can point at `/Post` or `/Product`.
                        let mut items = Vec::new();
                        let mut groups: std::collections::BTreeMap<String, Vec<Value>> =
                            std::collections::BTreeMap::new();
                        let mut remove_keys = Vec::new();
                        for (k, v) in top.iter() {
                            if let Some((typename, _)) = k.split_once(':')
                                && !k.starts_with('$')
                                && let Value::Object(obj) = v
                            {
                                if obj.contains_key("title") || obj.contains_key("url_slug") {
                                    items.push(v.clone());
                                }
                                if !typename.is_empty() {
                                    groups
                                        .entry(typename.to_owned())
                                        .or_default()
                                        .push(v.clone());
                                }
                                remove_keys.push(k.clone());
                            }
                        }
                        for k in remove_keys {
                            top.remove(&k);
                        }
                        if !items.is_empty() {
                            top.insert("items".into(), Value::Array(items));
                        }
                        for (typename, records) in groups {
                            top.entry(typename).or_insert(Value::Array(records));
                        }
                    }
                    payloads.insert(ident.to_owned(), val);
                }
            }
            let mut search_from = 0;
            while let Some(pos) = raw[search_from..].find("__next_f.push(") {
                let start = search_from + pos + "__next_f.push(".len();
                let rhs = raw[start..].trim_start();
                if rhs.starts_with('[') {
                    let mut stream = serde_json::Deserializer::from_str(rhs).into_iter::<Value>();
                    if let Some(Ok(Value::Array(arr))) = stream.next()
                        && let Some(Value::String(chunk)) = arr.get(1)
                    {
                        next_f_stream.push_str(chunk);
                    }
                }
                search_from = start;
            }
        }
    }
    if !next_f_stream.is_empty() {
        let mut next_f_obj = serde_json::Map::new();
        for line in next_f_stream.lines().take(500) {
            let Some((_, rhs)) = line.split_once(':') else {
                continue;
            };
            let rhs = rhs.trim();
            if (rhs.starts_with('[') || rhs.starts_with('{'))
                && let Ok(parsed) = serde_json::from_str::<Value>(rhs)
            {
                collect_next_f_props(&parsed, &mut next_f_obj, 0);
            }
        }
        if !next_f_obj.is_empty() {
            payloads.insert("__NEXT_F__".to_owned(), Value::Object(next_f_obj));
        }
    }
    payloads
}

/// Extract a specific SSR JSON payload (or `"auto"` for the first available) and optional JSON pointer.
pub fn extract_ssr_json(html: &str, source: &str, pointer: &str) -> Option<Value> {
    let map = extract_ssr_map(html);
    if map.is_empty() {
        return None;
    }
    let root = if source.is_empty() || source == "auto" {
        map.get("__NEXT_DATA__")
            .or_else(|| map.get("__remixContext"))
            .or_else(|| map.get("__NEXT_F__"))
            .or_else(|| map.get("__APOLLO_STATE__"))
            .or_else(|| map.get("__NUXT_DATA__"))
            .or_else(|| map.get("__INITIAL_STATE__"))
            .or_else(|| map.get("__PRELOADED_STATE__"))
            .or_else(|| map.get("__STATE__"))
            .or_else(|| map.get("INIT_DATA"))
            .or_else(|| map.values().next())?
    } else {
        map.get(source).or_else(|| {
            if source == "ld+json" {
                None
            } else {
                map.get("__NEXT_DATA__")
                    .or_else(|| map.get("__remixContext"))
                    .or_else(|| map.get("__NEXT_F__"))
                    .or_else(|| map.get("__APOLLO_STATE__"))
                    .or_else(|| map.get("__NUXT_DATA__"))
                    .or_else(|| map.get("__INITIAL_STATE__"))
            }
        })?
    };
    resolve_pointer(root, pointer).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extraction_refuses_missing_or_ambiguous_required_fields() {
        let extraction: Extraction = serde_json::from_value(json!({"items":"article","fields":{"title":{"selector":"h2","required":true},"url":{"selector":"a","attribute":"href","absolute_url":true}}})).unwrap();
        assert!(
            extraction
                .extract(
                    "<article><h2>One</h2><h2>Two</h2></article>",
                    "https://example.test/list"
                )
                .is_none()
        );
        let value = extraction
            .extract(
                "<article><h2>A\n title</h2><a href='javascript:alert(1)'>Link</a></article>",
                "https://example.test/list",
            )
            .unwrap();
        assert_eq!(value["items"][0]["title"], "A title");
        assert!(value["items"][0]["url"].is_null());
        let value = extraction.extract("<article><h2>A\n title<script>private-value</script><span hidden>hidden-value</span><style>private-style</style></h2></article>", "https://example.test/list").unwrap();
        assert_eq!(value["items"][0]["title"], "A title");
    }

    #[test]
    fn identity_fields_refuse_ambiguous_long_or_empty_values() {
        let identity = Field {
            selector: "h2".into(),
            attribute: String::new(),
        };
        assert_eq!(
            identity.extract(&parse_document("<h2>Mina\n Park</h2>")),
            Some("Mina Park".into())
        );
        let long_page = parse_document(&format!("<h2>{}</h2>", "Book title ".repeat(100)));
        assert!(identity.extract(&long_page).is_none());
        assert!(identity.is_present(&long_page));
        assert!(!identity.is_present(&parse_document("<h2>One</h2><h2>Two</h2>")));
        let meta = Field {
            selector: "meta[name=\"current-person\"]".into(),
            attribute: "content".into(),
        };
        assert!(
            meta.extract(&parse_document("<meta name='current-person' content=''>"))
                .is_none()
        );
    }

    #[test]
    fn extraction_rejects_credential_named_outputs() {
        let extraction: Extraction = serde_json::from_value(
            json!({"fields":{"csrf_token":{"selector":"meta","attribute":"content"}}}),
        )
        .unwrap();
        assert!(!extraction.valid());
        assert!(!selector_valid("div[["));
        assert!(selector_valid("article > h2.title"));
    }

    #[test]
    fn ssr_extraction_supports_remix_context_nextjs_flight_and_merged_ld_json() {
        let html = r#"
            <script type="application/ld+json">{"@context":"https://schema.org","@type":"WebSite","potentialAction":{"target":{"urlTemplate":"https://example.com/search?q={q}"}}}</script>
            <script type="application/ld+json">{"@context":"https://schema.org","@type":"ItemList","itemListElement":[{"name":"MacBook Pro"}]}</script>
            <script>window.__remixContext = {"state":{"loaderData":{"root":{"lang":"ko"},"routes/kr.search._index":{"articles":[{"title":"M3 Max; 16인치"}]}}}}; console.log("done");</script>
            <script>self.__next_f.push([1,"6:[\"$\",\"$L10\",null,{\"children\":[\"$\",\"$L13\",null,{\"data\":[{\"id\":\"p1\",\"title\":\"Hello\"}]}]}]\n"])</script>
            <script>window.__APOLLO_STATE__ = {"ROOT_QUERY":{"searchPosts":{"count":1}},"Post:abc-123":{"id":"abc-123","title":"Rust Guide","url_slug":"rust-guide"}};</script>
        "#;
        let map = extract_ssr_map(html);
        for key in [
            "__remixContext",
            "__NEXT_F__",
            "ld+json",
            "__APOLLO_STATE__",
        ] {
            assert!(map.contains_key(key), "{key}");
        }
        assert_eq!(
            extract_ssr_json(html, "__remixContext", "/routeData/articles/0/title"),
            Some(json!("M3 Max; 16인치"))
        );
        assert_eq!(
            extract_ssr_json(
                html,
                "__remixContext",
                "/state/loaderData/routes/kr.search._index/articles/0/title"
            ),
            Some(json!("M3 Max; 16인치"))
        );
        assert_eq!(
            extract_ssr_json(html, "__NEXT_F__", "/data/0/title"),
            Some(json!("Hello"))
        );
        assert_eq!(
            extract_ssr_json(html, "ld+json", "/itemListElement/0/name"),
            Some(json!("MacBook Pro"))
        );
        assert_eq!(
            extract_ssr_json(html, "__APOLLO_STATE__", "/items/0/title"),
            Some(json!("Rust Guide"))
        );
        assert_eq!(
            extract_ssr_json(html, "__APOLLO_STATE__", "/Post/0/url_slug"),
            Some(json!("rust-guide"))
        );
        assert_eq!(extract_ssr_json("<p>no scripts</p>", "auto", ""), None);
    }
}
