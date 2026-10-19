//! Bounded structural HTML evidence and declarative extraction, without script execution.
//!
//! Derived from Hycli `src/html.rs` (Apache-2.0). Changes: only the parts that
//! execute a declared extraction or verify a response were kept (`Field`,
//! `Extraction`, embedded SSR JSON). Evidence collection for site preparation
//! (`structure`, `fields`, `forms`, `infer_html_extraction`, `nav_list_routes`)
//! will live with the preparation layer. A site-specific rule for one
//! marketplace's Remix payload was removed. `resolve_pointer` moved to
//! `itda_types::json`.
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use scraper::Html as Document;

/// True when `selector` is a CSS selector this crate can evaluate.
pub fn selector_valid(selector: &str) -> bool {
    selector.len() <= 500 && Selector::parse(selector).is_ok()
}

/// Parse a full HTML document for [`Field::extract`] and [`Field::is_present`].
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

fn stable_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

fn element_text(element: scraper::ElementRef<'_>) -> String {
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
}
