//! What a fetched page shows about itself, for drafting a definition: repeated
//! records, tables, forms, feeds and pagination. Nothing here runs a script or
//! keeps a field value; only names, selectors and shapes come out.
//!
//! Derived from Hycli `src/html/extract.rs` (`infer_html_extraction`) and
//! `src/html/forms.rs` (`forms`) (Apache-2.0). Changes: tables with header
//! rows are recognised, form controls carry their type and whether they are
//! hidden or secret-named, values never leave the page, and feeds and
//! pagination links are collected for the same draft.
use crate::{Extraction, Field, OutputField, element_text, stable_name};
use itda_types::util::secret_field;
use scraper::{ElementRef, Html, Selector};
use std::collections::BTreeMap;

/// Record selectors that list pages use, tried before the page's own classes.
const KNOWN_RECORDS: &[&str] = &[
    "tr.athing",
    "article",
    "li.story",
    "div.post-item",
    "div.search-result",
    "div.result",
    "div.item",
    "li.item",
    "li.post",
];
const RECORD_WORDS: &[&str] = &[
    "row", "item", "topic", "post", "card", "story", "result", "entry", "article", "thread",
    "notice", "list",
];
const PAGE_KEYS: &[&str] = &[
    "page", "p", "pg", "pageno", "page_no", "pagenum", "offset", "start",
];

/// One control of a form, without its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub name: String,
    /// `string`, `int` or `bool`, as definitions declare inputs.
    pub kind: &'static str,
    pub required: bool,
    pub hidden: bool,
    /// The name looks like a token: it comes from the page, never from an agent.
    pub secret: bool,
    /// Only a hidden control that is not a token keeps its value: the form
    /// always sends it, so a definition needs it as a default.
    pub value: Option<String>,
}

/// The shape of a form: where it goes, how, and what it asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    /// Resolved against the page address; empty when it could not be.
    pub action: String,
    pub method: String,
    pub controls: Vec<Control>,
    /// A sign-in form; a person fills it in, never an agent.
    pub password: bool,
    pub multipart: bool,
}

/// A feed the page links to with `<link rel="alternate">`.
#[derive(Clone, Debug, PartialEq)]
pub struct Feed {
    pub url: String,
    pub title: String,
}

/// Repeated records on the page: a table with a header row, or a list of
/// similar elements with a title and a link. `None` when nothing repeats.
pub fn infer_extraction(html: &str, base: &str) -> Option<Extraction> {
    let document = Html::parse_document(html);
    url::Url::parse(base).ok()?;
    table_extraction(&document)
        .into_iter()
        .chain(record_extraction(&document))
        .find(|extraction| extraction.valid() && extraction.extract(html, base).is_some())
}

/// The first table with a header row and at least two body rows.
fn table_extraction(document: &Html) -> Option<Extraction> {
    let tables = Selector::parse("table").ok()?;
    let header_cells = Selector::parse("thead th, tr:first-child th").ok()?;
    let body_rows = Selector::parse("tbody tr, tr").ok()?;
    let cells = Selector::parse("td").ok()?;
    for (index, table) in document.select(&tables).take(20).enumerate() {
        let headers: Vec<String> = table
            .select(&header_cells)
            .map(|cell| field_name(&element_text(cell)))
            .collect();
        if headers.len() < 2 || headers.iter().filter(|h| !h.is_empty()).count() < 2 {
            continue;
        }
        let rows: Vec<ElementRef<'_>> = table
            .select(&body_rows)
            .filter(|row| row.select(&cells).count() >= headers.len().min(2))
            .collect();
        if rows.len() < 2 {
            continue;
        }
        let mut fields = BTreeMap::new();
        let mut seen = BTreeMap::new();
        for (column, header) in headers.iter().enumerate() {
            if header.is_empty() || secret_field(header) {
                continue;
            }
            let count = seen.entry(header.clone()).or_insert(0usize);
            *count += 1;
            let name = if *count == 1 {
                header.clone()
            } else {
                format!("{header}-{count}")
            };
            fields.insert(
                name,
                OutputField {
                    selector: format!("td:nth-child({})", column + 1),
                    attribute: String::new(),
                    required: false,
                    absolute_url: false,
                },
            );
        }
        if let Some(link) = pick_field(&rows, &[("a[href]", "href")], true)
            && !fields.contains_key("link")
        {
            fields.insert("link".into(), link);
        }
        if fields.is_empty() {
            continue;
        }
        // The parser wraps rows in tbody; a header row outside thead lands
        // there too, so it is skipped by position.
        let has_thead = Selector::parse("thead")
            .ok()
            .is_some_and(|s| table.select(&s).next().is_some());
        let rows = if has_thead {
            "tbody tr"
        } else {
            "tbody tr:not(:first-child)"
        };
        let items = if index == 0 {
            format!("table {rows}")
        } else {
            format!("table:nth-of-type({}) {rows}", index + 1)
        };
        return Some(Extraction {
            items,
            fields,
            page: BTreeMap::new(),
            limit: 100,
        });
    }
    None
}

/// Deterministically pick a repeating record element with a title and a link,
/// as Hycli did, from known selectors and the page's own class names.
fn record_extraction(document: &Html) -> Option<Extraction> {
    let mut candidates: Vec<String> = KNOWN_RECORDS.iter().map(|s| (*s).to_owned()).collect();
    if let Ok(scan) = Selector::parse("main *[class], body *[class]") {
        for element in document.select(&scan).take(600) {
            let tag = element.value().name();
            if !matches!(tag, "div" | "li" | "article" | "tr") {
                continue;
            }
            let class = element.value().classes().find(|class| {
                class.len() <= 40
                    && class
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    && RECORD_WORDS
                        .iter()
                        .any(|word| class.to_ascii_lowercase().contains(word))
            });
            if let Some(class) = class {
                let candidate = format!("{tag}.{class}");
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
    }
    for items in candidates {
        let Ok(selector) = Selector::parse(&items) else {
            continue;
        };
        let records: Vec<ElementRef<'_>> = document.select(&selector).take(40).collect();
        if records.len() < 3 {
            continue;
        }
        let mut fields = BTreeMap::new();
        if let Some(field) = pick_field(&records, &[("", "data-id"), ("", "id")], false) {
            fields.insert("id".into(), field);
        }
        let Some(title) = pick_field(
            &records,
            &[
                ("h1", ""),
                ("h2", ""),
                ("h3", ""),
                ("h4", ""),
                (".titleline > a", ""),
                (".title a", ""),
                ("a[class*='title']", ""),
                (".title", ""),
                ("a", ""),
            ],
            false,
        ) else {
            continue;
        };
        let Some(link) = pick_field(
            &records,
            &[
                (".titleline > a", "href"),
                ("h1 a", "href"),
                ("h2 a", "href"),
                ("h3 a", "href"),
                (".title a", "href"),
                ("a[class*='title']", "href"),
                ("a[href]", "href"),
            ],
            true,
        ) else {
            continue;
        };
        fields.insert("title".into(), title);
        fields.insert("link".into(), link);
        for (name, options) in [
            (
                "summary",
                &[
                    (".summary", ""),
                    (".description", ""),
                    (".excerpt", ""),
                    (".snippet", ""),
                    ("p", ""),
                ][..],
            ),
            (
                "meta",
                &[
                    (".subtext", ""),
                    (".meta", ""),
                    (".byline", ""),
                    (".info", ""),
                ][..],
            ),
            (
                "published_at",
                &[("time[datetime]", "datetime"), ("time", "")][..],
            ),
        ] {
            if let Some(field) = pick_field(&records, options, false) {
                fields.insert(name.into(), field);
            }
        }
        return Some(Extraction {
            items,
            fields,
            page: BTreeMap::new(),
            limit: 50,
        });
    }
    None
}

/// The first selector that finds one non-empty value in most records.
fn pick_field(
    records: &[ElementRef<'_>],
    options: &[(&str, &str)],
    absolute_url: bool,
) -> Option<OutputField> {
    let needed = (records.len() * 4).div_ceil(5).max(2);
    for &(selector, attribute) in options {
        let parsed = match selector {
            "" => None,
            _ => Some(Selector::parse(selector).ok()?),
        };
        let hits = records
            .iter()
            .filter(|record| {
                let target = match &parsed {
                    Some(selector) => {
                        let mut matches = record.select(selector);
                        let first = matches.next();
                        if matches.next().is_some() {
                            None
                        } else {
                            first
                        }
                    }
                    None => Some(**record),
                };
                target.is_some_and(|element| {
                    let value = if attribute.is_empty() {
                        element_text(element)
                    } else {
                        element
                            .value()
                            .attr(attribute)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    !value.trim().is_empty() && value.len() <= 16_384
                })
            })
            .count();
        if hits >= needed {
            return Some(OutputField {
                selector: selector.to_owned(),
                attribute: attribute.to_owned(),
                required: false,
                absolute_url,
            });
        }
    }
    None
}

/// A selector that identifies the page: its one `h1`, or else its `title`.
pub fn title_field(html: &str) -> Option<Field> {
    let document = Html::parse_document(html);
    ["h1", "title"].into_iter().find_map(|selector| {
        let field = Field {
            selector: selector.into(),
            attribute: String::new(),
        };
        field.is_present(&document).then_some(field)
    })
}

/// Every form on the page, with its controls' names and types but no values.
pub fn forms(html: &str, base: &str) -> Vec<Form> {
    let document = Html::parse_document(html);
    let base = url::Url::parse(base).ok();
    let (Ok(form_selector), Ok(control_selector)) = (
        Selector::parse("form"),
        Selector::parse("input[name], select[name], textarea[name]"),
    ) else {
        return Vec::new();
    };
    document
        .select(&form_selector)
        .take(40)
        .map(|form| {
            let action = form.value().attr("action").unwrap_or_default().trim();
            let action = base
                .as_ref()
                .and_then(|base| base.join(action).ok())
                .map(|url| {
                    let mut url = url;
                    url.set_fragment(None);
                    url.to_string()
                })
                .unwrap_or_default();
            let method = form
                .value()
                .attr("method")
                .unwrap_or("GET")
                .trim()
                .to_ascii_uppercase();
            let method = if method == "POST" {
                method
            } else {
                "GET".into()
            };
            let multipart = form
                .value()
                .attr("enctype")
                .is_some_and(|e| e.contains("multipart"));
            let mut password = false;
            let mut controls: Vec<Control> = Vec::new();
            for control in form.select(&control_selector).take(60) {
                let Some(name) = control.value().attr("name").filter(|n| stable_name(n)) else {
                    continue;
                };
                let tag = control.value().name();
                let kind = control
                    .value()
                    .attr("type")
                    .unwrap_or(if tag == "input" { "text" } else { tag })
                    .to_ascii_lowercase();
                match kind.as_str() {
                    "password" => {
                        password = true;
                        continue;
                    }
                    "submit" | "button" | "reset" | "image" | "file" => continue,
                    _ => {}
                }
                if controls.iter().any(|c| c.name == name) {
                    continue;
                }
                let (hidden, secret) = (kind == "hidden", secret_field(name));
                let value = control
                    .value()
                    .attr("value")
                    .filter(|_| hidden && !secret)
                    .filter(|v| v.len() <= 200 && !v.contains(['\r', '\n']))
                    .map(str::to_owned);
                controls.push(Control {
                    name: name.to_owned(),
                    kind: match kind.as_str() {
                        "number" | "range" => "int",
                        "checkbox" => "bool",
                        _ => "string",
                    },
                    required: control.value().attr("required").is_some(),
                    hidden,
                    secret,
                    value,
                });
            }
            Form {
                action,
                method,
                controls,
                password,
                multipart,
            }
        })
        .collect()
}

/// RSS and Atom feeds the page links to.
pub fn feeds(html: &str, base: &str) -> Vec<Feed> {
    let document = Html::parse_document(html);
    let Ok(base) = url::Url::parse(base) else {
        return Vec::new();
    };
    let Ok(selector) = Selector::parse("link[rel=\"alternate\"][href]") else {
        return Vec::new();
    };
    document
        .select(&selector)
        .filter(|link| {
            link.value()
                .attr("type")
                .is_some_and(|t| t.contains("rss") || t.contains("atom"))
        })
        .filter_map(|link| {
            let url = base.join(link.value().attr("href")?).ok()?;
            matches!(url.scheme(), "http" | "https").then(|| Feed {
                url: url.to_string(),
                title: link
                    .value()
                    .attr("title")
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            })
        })
        .take(5)
        .collect()
}

/// The query key the page's own links use to turn pages (`?page=2`), when
/// they point at the same path.
pub fn page_key(html: &str, page: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let page = url::Url::parse(page).ok()?;
    let selector = Selector::parse("a[href]").ok()?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for link in document.select(&selector).take(2000) {
        let Ok(url) = page.join(link.value().attr("href").unwrap_or_default()) else {
            continue;
        };
        if url.origin() != page.origin() || url.path() != page.path() {
            continue;
        }
        for (key, value) in url.query_pairs() {
            if PAGE_KEYS.contains(&key.to_ascii_lowercase().as_str())
                && value.parse::<u64>().is_ok()
            {
                *counts.entry(key.into_owned()).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(key, _)| key)
}

/// Does the page link to an OpenAPI description? The address, if so.
pub fn openapi_link(html: &str, base: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let base = url::Url::parse(base).ok()?;
    let selector = Selector::parse("link[href], a[href]").ok()?;
    document.select(&selector).take(2000).find_map(|link| {
        let href = link.value().attr("href")?;
        let rel = link.value().attr("rel").unwrap_or_default();
        let lower = href.to_ascii_lowercase();
        let described = rel.contains("service-desc")
            || lower.ends_with("openapi.json")
            || lower.ends_with("openapi.yaml")
            || lower.ends_with("swagger.json");
        described
            .then(|| base.join(href).ok())
            .flatten()
            .map(|u| u.to_string())
    })
}

/// True when the page is mostly scripts with little text: an app shell that
/// draws itself in the browser, which a plain fetch cannot see into.
pub fn script_shell(html: &str) -> bool {
    let document = Html::parse_document(html);
    let text = element_text(document.root_element());
    let scripts = Selector::parse("script[src]")
        .map(|s| document.select(&s).count())
        .unwrap_or(0);
    text.len() < 400 && scripts >= 1
}

/// A header cell's text as a field name: `Posted on` becomes `posted-on`.
fn field_name(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().take(60) {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://example.test/board";

    #[test]
    fn tables_with_headers_become_records() {
        let html = "<h1>Board</h1><table><thead><tr><th>No</th><th>Title</th><th>Posted on</th></tr></thead>
            <tbody><tr><td>1</td><td><a href='/n/1'>First</a></td><td>2026-01-01</td></tr>
            <tr><td>2</td><td><a href='/n/2'>Second</a></td><td>2026-01-02</td></tr></tbody></table>";
        let extraction = infer_extraction(html, BASE).unwrap();
        assert_eq!(extraction.items, "table tbody tr");
        assert_eq!(extraction.fields["title"].selector, "td:nth-child(2)");
        assert_eq!(extraction.fields["posted-on"].selector, "td:nth-child(3)");
        assert!(extraction.fields["link"].absolute_url);
        let value = extraction.extract(html, BASE).unwrap();
        assert_eq!(value["items"][1]["title"], "Second");
        assert_eq!(value["items"][1]["link"], "https://example.test/n/2");
    }

    #[test]
    fn repeated_cards_become_records() {
        let html = "<main>".to_owned()
            + &(1..=4)
                .map(|n| format!("<div class='post-card'><h2><a href='/p/{n}'>Post {n}</a></h2><p>Body {n}</p><time datetime='2026-01-0{n}'>Jan</time></div>"))
                .collect::<String>()
            + "</main>";
        let extraction = infer_extraction(&html, BASE).unwrap();
        assert_eq!(extraction.items, "div.post-card");
        assert_eq!(extraction.fields["title"].selector, "h2");
        assert_eq!(extraction.fields["published_at"].attribute, "datetime");
        assert!(infer_extraction("<p>one</p><p>two</p>", BASE).is_none());
    }

    #[test]
    fn forms_keep_names_and_types_but_never_values() {
        let html = r#"<form action="/search" method="get"><input name="q" required><input type="hidden" name="board" value="3"><select name="sort"><option>new</option></select><input type="submit" name="go"></form>
            <form action="https://other.test/x" method="POST" enctype="multipart/form-data"><input name="title"><input type="file" name="upload"></form>
            <form action="/login" method="post"><input name="user"><input type="password" name="pw"><input type="hidden" name="_csrf" value="abc"></form>"#;
        let forms = forms(html, BASE);
        assert_eq!(forms.len(), 3);
        assert_eq!(forms[0].action, "https://example.test/search");
        assert_eq!(forms[0].method, "GET");
        let names: Vec<&str> = forms[0].controls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["q", "board", "sort"]);
        assert!(forms[0].controls[0].required);
        assert!(forms[0].controls[1].hidden);
        assert_eq!(forms[0].controls[1].value.as_deref(), Some("3"));
        assert!(forms[1].multipart);
        assert!(forms[2].password);
        assert!(
            forms[2]
                .controls
                .iter()
                .any(|c| c.name == "_csrf" && c.secret)
        );
        assert!(!format!("{forms:?}").contains("abc"));
    }

    #[test]
    fn feeds_pagination_and_titles_are_found() {
        let html = r#"<head><title>Board</title><link rel="alternate" type="application/rss+xml" title="Posts" href="/feed.xml"><link rel="service-desc" href="/openapi.json"></head>
            <body><h1>Notices</h1><a href="/board?page=2">2</a><a href="/board?page=3">3</a><a href="/other?page=9">x</a></body>"#;
        assert_eq!(feeds(html, BASE)[0].url, "https://example.test/feed.xml");
        assert_eq!(feeds(html, BASE)[0].title, "Posts");
        assert_eq!(page_key(html, BASE).as_deref(), Some("page"));
        assert_eq!(title_field(html).unwrap().selector, "h1");
        assert_eq!(
            title_field("<title>Only</title>").unwrap().selector,
            "title"
        );
        assert_eq!(
            openapi_link(html, BASE).as_deref(),
            Some("https://example.test/openapi.json")
        );
        assert!(script_shell(
            "<div id=root></div><script src=/app.js></script>"
        ));
        assert!(!script_shell(html));
    }
}
