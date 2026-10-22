//! What a fetched page shows about itself, for drafting a definition: repeated
//! records, tables, the main content of a page that is one thing, forms,
//! feeds and pagination. Nothing here runs a script or keeps a field value;
//! only names, selectors and shapes come out.
//!
//! Derived from Hycli `src/html/extract.rs` (`infer_html_extraction`) and
//! `src/html/forms.rs` (`forms`) (Apache-2.0). Changes: tables with header
//! rows are recognised, form controls carry their type and whether they are
//! hidden or secret-named, values never leave the page, and feeds and
//! pagination links are collected for the same draft.
use crate::{Extraction, Field, OutputField, element_text, stable_name};
use itda_types::util::secret_field;
use scraper::{ElementRef, Html, Selector};
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

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
    /// The name of the form's file control, when it has one: the form
    /// sends a file from the person's computer.
    pub file: Option<String>,
}

/// A link the page labels as an attachment to download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Resolved against the page address.
    pub url: String,
    pub text: String,
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
        .chain(generic_extraction(&document))
        .find(|extraction| extraction.valid() && extraction.extract(html, base).is_some())
}

/// Elements that repeat as records, and the page furniture they are never in.
const RECORD_TAGS: &[&str] = &[
    "li", "tr", "article", "div", "section", "dd", "dt", "p", "a",
];
const FURNITURE: &[&str] = &["nav", "aside", "header", "footer"];
/// At most this many fields a record read declares.
const MAX_RECORD_FIELDS: usize = 12;
/// How many records are looked into to find where their fields sit.
const SAMPLE: usize = 20;

/// Any run of three or more siblings of one shape, outside navigation,
/// whose members each hold a title at the same place: a link, a heading
/// or a named text, or the record is itself a link. A list page the known
/// selectors do not name. The title and link are read from the title's
/// own path, found in the page rather than guessed; the rest of the fields
/// come from what the records share (`shared_fields`).
fn generic_extraction(document: &Html) -> Option<Extraction> {
    let everything = Selector::parse("body *").ok()?;
    let mut candidates: Vec<(ElementRef<'_>, String, Vec<ElementRef<'_>>)> = Vec::new();
    for parent in document.select(&everything).take(40_000) {
        if furniture(parent) {
            continue;
        }
        let children: Vec<ElementRef<'_>> =
            parent.children().filter_map(ElementRef::wrap).collect();
        let without = ads_excluded(&children).unwrap_or_default();
        let mut groups: BTreeMap<String, Vec<ElementRef<'_>>> = BTreeMap::new();
        for child in children {
            if RECORD_TAGS.contains(&child.value().name()) && !ad_slot(child) {
                groups.entry(segment(child)).or_default().push(child);
            }
        }
        for (shape, records) in groups {
            // Bare paragraphs and bare links are prose and menus, never
            // records; a class the site gave them says otherwise.
            if records.len() >= 3
                && (shape.contains('.') || (shape != "p" && shape != "a"))
                && same_shape(&records)
            {
                candidates.push((parent, format!("{shape}{without}"), records));
            }
        }
    }
    // Longest runs first, so that a run that cannot beat the best found so
    // far is skipped before its records are looked into.
    candidates.sort_by_key(|(_, _, records)| std::cmp::Reverse(records.len()));
    let mut best: Option<(usize, Extraction)> = None;
    for (parent, shape, records) in candidates {
        let most = records.len().min(100) * 120;
        if best.as_ref().is_some_and(|(top, _)| *top >= most) {
            continue;
        }
        let Some((items, records)) = items_selector(document, parent, &shape, records) else {
            continue;
        };
        let Some((title, link, text_len, linked)) = record_title(&records) else {
            continue;
        };
        let score = records.len().min(100) * text_len.min(120);
        if best.as_ref().is_some_and(|(top, _)| *top >= score) {
            continue;
        }
        let mut fields = BTreeMap::new();
        if let Some(field) = pick_field(&records, &[("", "data-id"), ("", "id")], false) {
            fields.insert("id".into(), field);
        }
        fields.insert("title".into(), title);
        if let Some(link) = link {
            fields.insert("link".into(), link);
        }
        shared_fields(&records, &mut fields);
        // A title that is a link is a record by itself; a heading or a
        // link that is the whole record needs one more thing to read,
        // or it is a tag cloud or a list of section names.
        let known = ["id", "title", "link"];
        if !linked && !fields.keys().any(|name| !known.contains(&name.as_str())) {
            continue;
        }
        best = Some((
            score,
            Extraction {
                items,
                fields,
                page: BTreeMap::new(),
                limit: 100,
            },
        ));
    }
    best.map(|(_, extraction)| extraction)
}

/// The tags of an element's children, in order: two records of one shape
/// hold the same kinds of things in the same order.
fn signature(element: ElementRef<'_>) -> String {
    element
        .children()
        .filter_map(ElementRef::wrap)
        .take(8)
        .map(|child| child.value().name())
        .collect::<Vec<_>>()
        .join(",")
}

/// Most of the records (half or more) are built the same way inside.
fn same_shape(records: &[ElementRef<'_>]) -> bool {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for record in records {
        *counts.entry(signature(*record)).or_default() += 1;
    }
    counts
        .values()
        .max()
        .is_some_and(|top| top * 2 >= records.len())
}

/// The title of these records and, when it has one, their link: the one
/// link most records hold at the same place, else a heading, a text named
/// as a title, or the record itself when it is a link. Also the average
/// title length and whether the title is a link.
fn record_title(
    records: &[ElementRef<'_>],
) -> Option<(OutputField, Option<OutputField>, usize, bool)> {
    let text = |selector: &str| OutputField {
        selector: selector.to_owned(),
        attribute: String::new(),
        required: false,
        absolute_url: false,
    };
    let href = |selector: &str| OutputField {
        selector: selector.to_owned(),
        attribute: "href".into(),
        required: false,
        absolute_url: true,
    };
    if let Some((path, text_len)) = title_path(records) {
        return Some((text(&path), Some(href(&path)), text_len, true));
    }
    let is_heading =
        |e: ElementRef<'_>| matches!(e.value().name(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    let is_titled = |e: ElementRef<'_>| {
        e.value().classes().any(|class| {
            let lower = class.to_ascii_lowercase();
            ["title", "headline", "subject", "name"]
                .iter()
                .any(|word| lower.contains(word))
        })
    };
    // A quick look before the paths are worked out: most runs of siblings
    // hold neither a heading nor a named title.
    let has = |pick: &dyn Fn(ElementRef<'_>) -> bool| {
        records.iter().take(3).any(|record| {
            record
                .descendants()
                .filter_map(ElementRef::wrap)
                .take(300)
                .any(pick)
        })
    };
    let heading = if has(&is_heading) {
        shared_paths(records, is_heading).into_iter().next()
    } else {
        None
    }
    .or_else(|| {
        if has(&is_titled) {
            shared_paths(records, is_titled).into_iter().next()
        } else {
            None
        }
    });
    let anchors = records
        .iter()
        .all(|record| record.value().name() == "a" && record.value().attr("href").is_some());
    if let Some((path, text_len)) = heading {
        let link =
            pick_field(records, &[("a[href]", "href")], true).or_else(|| anchors.then(|| href("")));
        return Some((text(&path), link, text_len, false));
    }
    if anchors {
        let texts: Vec<usize> = records
            .iter()
            .map(|record| element_text(*record).trim().len())
            .filter(|len| *len > 0 && *len <= 300)
            .collect();
        if texts.len() >= needed(records.len()) {
            let average = texts.iter().sum::<usize>() / texts.len();
            return Some((text(""), Some(href("")), average, false));
        }
    }
    None
}

/// How many records a field must be in: four of five, at least two.
fn needed(records: usize) -> usize {
    (records * 4).div_ceil(5).max(2)
}

/// The path, relative to a record, of an element the predicate picks and
/// that most records hold once at the same place with a text that is not
/// too long; with the average length of that text, best first.
fn shared_paths(
    records: &[ElementRef<'_>],
    pick: impl Fn(ElementRef<'_>) -> bool,
) -> Vec<(String, usize)> {
    let mut paths: Vec<String> = Vec::new();
    for record in records.iter().take(5) {
        for element in record.descendants().filter_map(ElementRef::wrap).take(300) {
            if element.id() == record.id() || !pick(element) {
                continue;
            }
            let path = relative_path(*record, element);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    // A sample of the records says where the field is; the read itself
    // runs on all of them.
    let records = &records[..records.len().min(SAMPLE)];
    let needed = needed(records.len());
    let mut found: Vec<(String, usize, usize)> = paths
        .into_iter()
        .take(80)
        .filter_map(|path| {
            let selector = Selector::parse(&path).ok()?;
            let texts: Vec<usize> = records
                .iter()
                .filter_map(|record| {
                    let text = element_text(one(*record, &selector)?);
                    let text = text.trim();
                    (!text.is_empty() && text.len() <= 300).then_some(text.len())
                })
                .collect();
            (texts.len() >= needed).then(|| {
                let average = texts.iter().sum::<usize>() / texts.len();
                (path, texts.len(), average)
            })
        })
        .collect();
    found.sort_by_key(|(path, hits, _)| (std::cmp::Reverse(*hits), path.len()));
    found
        .into_iter()
        .map(|(path, _, average)| (path, average))
        .collect()
}

/// The element's path below the record: `div.body > span.meta > a:nth-of-type(2)`,
/// with a position only on the last step and only when it has no class.
fn relative_path(record: ElementRef<'_>, element: ElementRef<'_>) -> String {
    let mut path = Vec::new();
    let mut node = Some(element);
    while let Some(e) = node.filter(|e| e.id() != record.id()) {
        path.insert(0, segment(e));
        node = e.parent().and_then(ElementRef::wrap);
    }
    if let Some(last) = path.last_mut()
        && !last.contains('.')
    {
        let position = element
            .prev_siblings()
            .filter_map(ElementRef::wrap)
            .filter(|e| e.value().name() == element.value().name())
            .count()
            + 1;
        let siblings = position
            + element
                .next_siblings()
                .filter_map(ElementRef::wrap)
                .filter(|e| e.value().name() == element.value().name())
                .count();
        if siblings > 1 {
            *last = format!("{last}:nth-of-type({position})");
        }
    }
    path.join(" > ")
}

/// The one element the selector picks inside the record, or nothing when
/// it picks none or several.
fn one<'a>(record: ElementRef<'a>, selector: &Selector) -> Option<ElementRef<'a>> {
    let mut matches = record.select(selector);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

/// Fields most records share beyond the title and link: a summary, a
/// byline and a date where pages usually put them, then whatever else sits
/// at one place in most records under a class the site named (`score`,
/// `author`, `release__version`), deepest first, and a text that reads as
/// a date when no `time` says when.
fn shared_fields(records: &[ElementRef<'_>], fields: &mut BTreeMap<String, OutputField>) {
    let records = &records[..records.len().min(SAMPLE)];
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
        if fields.contains_key(name) {
            continue;
        }
        if let Some(field) = pick_field(records, options, false) {
            fields.insert(name.into(), field);
        }
    }
    let title = fields.get("title").cloned();
    let title_texts: Vec<String> = title
        .as_ref()
        .and_then(|title| Selector::parse(&title.selector).ok().map(|s| (title, s)))
        .map(|(title, selector)| {
            records
                .iter()
                .map(|record| {
                    let element = if title.selector.is_empty() {
                        Some(*record)
                    } else {
                        one(*record, &selector)
                    };
                    element
                        .map(element_text)
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default();
    let record_class = records
        .first()
        .and_then(|record| record.value().classes().find(|class| selector_word(class)))
        .unwrap_or_default()
        .to_owned();
    let mut named = shared_paths(records, |e| e.value().classes().any(selector_word));
    // Deepest first: `span.score` inside `span.meta` is the field, and the
    // line that holds it is left to `meta`.
    named.sort_by_key(|(path, _)| std::cmp::Reverse(path.matches(" > ").count()));
    let mut kept: Vec<String> = fields.values().map(|f| f.selector.clone()).collect();
    for (path, _) in named {
        if fields.len() >= MAX_RECORD_FIELDS {
            break;
        }
        // Not a field already read, and not the line around one.
        if kept
            .iter()
            .any(|k| *k == path || k.starts_with(&format!("{path} > ")))
        {
            continue;
        }
        let Some(class) = path
            .rsplit(" > ")
            .next()
            .and_then(|last| last.split_once('.'))
            .map(|(_, class)| class.split(':').next().unwrap_or(class))
        else {
            continue;
        };
        let Ok(selector) = Selector::parse(&path) else {
            continue;
        };
        let texts: Vec<String> = records
            .iter()
            .map(|record| {
                one(*record, &selector)
                    .map(element_text)
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            })
            .collect();
        let same_as_title = title_texts
            .iter()
            .zip(&texts)
            .filter(|(a, b)| !a.is_empty() && a == b)
            .count();
        if same_as_title * 2 >= records.len() {
            continue;
        }
        let dated = texts.iter().filter(|t| date_like(t)).count() >= needed(records.len());
        let name = if dated && !fields.contains_key("published_at") {
            "published_at".to_owned()
        } else {
            match class_name(class, &record_class) {
                Some(name) if !fields.contains_key(&name) => name,
                _ => continue,
            }
        };
        kept.push(path.clone());
        fields.insert(
            name,
            OutputField {
                selector: path,
                attribute: String::new(),
                required: false,
                absolute_url: false,
            },
        );
    }
}

/// Classes that say where a thing sits, not what it is.
const LAYOUT_WORDS: &[&str] = &[
    "row",
    "col",
    "cols",
    "column",
    "wrap",
    "wrapper",
    "container",
    "inner",
    "outer",
    "left",
    "right",
    "center",
    "top",
    "bottom",
    "flex",
    "grid",
    "block",
    "inline",
    "box",
    "item",
    "card",
    "list",
    "link",
    "btn",
    "button",
    "icon",
    "img",
    "image",
    "thumb",
    "thumbnail",
    "avatar",
    "badge",
    "label",
    "hidden",
    "visible",
    "small",
    "large",
    "clearfix",
    "truncate",
    "sr-only",
    "line",
    "liner",
    "graphic",
    "node",
    "main",
];

/// A field name from a class the site gave the element: `release__version`
/// is `version`, `post-card-author` on a `post-card` is `author`; a layout
/// word, a build name or a credential-like name is nothing.
fn class_name(class: &str, record_class: &str) -> Option<String> {
    // `heading__fzQskI` is a build name: its last part was not written by
    // a person.
    let last = class.rsplit("__").next().unwrap_or(class);
    let upper = last.chars().filter(|c| c.is_ascii_uppercase()).count();
    let built = last.len() <= 8
        && last.contains(|c: char| c.is_ascii_alphabetic())
        && (last.contains(|c: char| c.is_ascii_digit())
            || (upper >= 2 && last.contains(|c: char| c.is_ascii_lowercase())));
    if built {
        return None;
    }
    // `post-card-author` on a `post-card` is `author`; so is `post-author`
    // next to a `post-title`.
    let stem = record_class
        .split("__")
        .next()
        .unwrap_or(record_class)
        .to_ascii_lowercase();
    let first = stem.split(['-', '_']).next().unwrap_or_default().to_owned();
    let mut name = last.to_ascii_lowercase();
    for prefix in ["js-", "is-", "has-", "u-", "c-"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.to_owned();
        }
    }
    for stem in [stem, first] {
        if !stem.is_empty()
            && let Some(rest) = name
                .strip_prefix(&format!("{stem}-"))
                .or_else(|| name.strip_prefix(&format!("{stem}_")))
        {
            name = rest.to_owned();
            break;
        }
    }
    let name = field_name(&name);
    (!name.is_empty()
        && name.len() <= 30
        && name.contains(|c: char| c.is_ascii_alphabetic())
        && !LAYOUT_WORDS.contains(&name.as_str())
        && !secret_field(&name)
        && stable_name(&name))
    .then_some(name)
}

/// A short text that says when: `2026-01-05`, `05/01/2026`, `Jan 5, 2026`,
/// `2026년 1월 5일`, `3 hours ago`, `2일 전`.
fn date_like(text: &str) -> bool {
    static DATE: OnceLock<regex::Regex> = OnceLock::new();
    let text = text.trim();
    if text.is_empty() || text.len() > 40 || !text.contains(|c: char| c.is_ascii_digit()) {
        return false;
    }
    let date = DATE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(\d{4}[-./년]\s?\d{1,2}([-./월]\s?\d{1,2})?)|(\d{1,2}[-./]\d{1,2}[-./]\d{2,4})|\b(jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\.?\s+\d{1,2}|\b\d{1,2}\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\b|\b\d+\s*(seconds?|minutes?|hours?|days?|weeks?|months?|years?|초|분|시간|일|주|개월|년)\s*(ago|전)\b|^\d{1,2}:\d{2}$",
        )
        .expect("constant")
    });
    date.is_match(text)
}

/// Inside navigation, a sidebar, a header or a footer: by tag, or by the
/// names pages give those areas when they use plain `div`s.
fn furniture(element: ElementRef<'_>) -> bool {
    std::iter::once(element)
        .chain(element.ancestors().filter_map(ElementRef::wrap))
        .any(furniture_self)
}

fn furniture_self(e: ElementRef<'_>) -> bool {
    const WORDS: &[&str] = &[
        "sidebar",
        "nav",
        "menu",
        "footer",
        "header",
        "breadcrumb",
        "toolbar",
    ];
    if FURNITURE.contains(&e.value().name()) {
        return true;
    }
    // Only containers are named after what they hold; a record's own id
    // (`431_request_header_fields_too_large`) says nothing about layout.
    if !matches!(e.value().name(), "div" | "section" | "ul" | "ol" | "menu") {
        return false;
    }
    let named = e.value().classes().chain(e.value().id()).any(|name| {
        let lower = name.to_ascii_lowercase();
        WORDS.iter().any(|word| lower.contains(word))
    });
    // A page-wide wrapper such as `div.flex-wrap-footer` holds the content
    // itself; a real sidebar or menu holds no heading or article.
    named
        && Selector::parse("h1, main, article")
            .is_ok_and(|content| e.select(&content).next().is_none())
}

/// An advertising slot, by its own classes, id or label (`AdHolder`,
/// `ad-banner`, `sponsored-post`, `aria-label="Sponsored"`): not one of the
/// page's records, and on some sites only there for some requests.
fn ad_slot(e: ElementRef<'_>) -> bool {
    let value = e.value();
    let labelled = ["aria-label", "data-label"].iter().any(|attr| {
        value.attr(attr).is_some_and(|label| {
            let lower = label.to_ascii_lowercase();
            lower.contains("sponsored") || lower.contains("advertisement")
        })
    });
    labelled || value.classes().chain(value.id()).any(ad_word)
}

fn ad_word(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "ad" | "ads" | "advert")
        || ["ad-", "ad_", "ads-", "ads_"]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
        || ["adholder", "sponsored", "advertisement"]
            .iter()
            .any(|word| lower.contains(word))
}

/// `:not(.class)` for every class that marks an ad among `elements`, so a
/// selector for the records beside them leaves the ads out. `None` when an
/// ad is marked only by its label and a class cannot leave it out.
fn ads_excluded(elements: &[ElementRef<'_>]) -> Option<String> {
    let mut classes: Vec<&str> = Vec::new();
    for element in elements.iter().filter(|e| ad_slot(**e)) {
        let marks: Vec<&str> = element
            .value()
            .classes()
            .filter(|class| ad_word(class) && selector_word(class))
            .collect();
        if marks.is_empty() {
            return None;
        }
        for class in marks {
            if !classes.contains(&class) {
                classes.push(class);
            }
        }
    }
    Some(
        classes
            .iter()
            .map(|class| format!(":not(.{class})"))
            .collect(),
    )
}

/// `tag.class` with the first class a selector can use, else just `tag`.
fn segment(element: ElementRef<'_>) -> String {
    let tag = element.value().name();
    element
        .value()
        .classes()
        .find(|class| selector_word(class))
        .map(|class| format!("{tag}.{class}"))
        .unwrap_or_else(|| tag.to_owned())
}

/// A class or id a selector can use and that will still be there tomorrow:
/// not a build hash such as `GridStyled-sc-772acff9-0` or `css-1x2y3z4`.
fn selector_word(word: &str) -> bool {
    let hashed = word.contains("sc-")
        || word.starts_with("css-")
        || word
            .split(['-', '_'])
            .any(|part| part.len() >= 5 && part.chars().all(|c| c.is_ascii_hexdigit()));
    !word.is_empty()
        && word.len() <= 40
        && !hashed
        && word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A selector that picks these records: the parent's path, made longer
/// until it matches exactly them, or else the same shape repeated under
/// several parents of one kind (a list per section), none of it furniture.
/// Returns the selector and every record it picks.
fn items_selector<'a>(
    document: &'a Html,
    parent: ElementRef<'a>,
    shape: &str,
    records: Vec<ElementRef<'a>>,
) -> Option<(String, Vec<ElementRef<'a>>)> {
    let mut path = Vec::new();
    let mut node = Some(parent);
    let mut spread: Option<(String, Vec<ElementRef<'a>>)> = None;
    for _ in 0..6 {
        let element = node?;
        let id = element.value().id().filter(|id| selector_word(id));
        match id {
            Some(id) => path.insert(0, format!("#{id}")),
            None => path.insert(0, segment(element)),
        }
        let candidate = format!("{} > {shape}", path.join(" > "));
        let matched: Vec<ElementRef<'a>> = Selector::parse(&candidate)
            .map(|selector| document.select(&selector).take(1001).collect())
            .unwrap_or_default();
        if matched.len() == records.len() {
            return Some((candidate, records));
        }
        let parallel = matched.len() <= 1000
            && spread
                .as_ref()
                .is_none_or(|(_, seen)| matched.len() < seen.len())
            && matched.iter().all(|m| {
                !furniture(*m)
                    && m.parent()
                        .and_then(ElementRef::wrap)
                        .is_some_and(|p| segment(p) == segment(parent))
            });
        if parallel {
            spread = Some((candidate, matched));
        }
        if id.is_some() {
            break;
        }
        node = element.parent().and_then(ElementRef::wrap);
    }
    spread
}

/// The path, relative to a record, of the one link most records have at
/// the same place, and the average length of its text.
fn title_path(records: &[ElementRef<'_>]) -> Option<(String, usize)> {
    let anchors = Selector::parse("a[href]").ok()?;
    let mut paths: Vec<String> = Vec::new();
    for record in records.iter().take(5) {
        for anchor in record.select(&anchors) {
            if element_text(anchor).trim().is_empty() {
                continue;
            }
            let mut path = Vec::new();
            let mut node = Some(anchor);
            while let Some(element) = node.filter(|e| e.id() != record.id()) {
                path.insert(0, segment(element));
                node = element.parent().and_then(ElementRef::wrap);
            }
            // Links without classes tell apart by position: `a:nth-of-type(2)`.
            let position = anchor
                .prev_siblings()
                .filter_map(ElementRef::wrap)
                .filter(|e| e.value().name() == "a")
                .count()
                + 1;
            let mut placed = path.clone();
            if let Some(last) = placed.last_mut() {
                *last = format!("{last}:nth-of-type({position})");
            }
            for path in [path.join(" > "), placed.join(" > ")] {
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
    }
    let needed = needed(records.len());
    paths
        .into_iter()
        .filter_map(|path| {
            let selector = Selector::parse(&path).ok()?;
            let texts: Vec<usize> = records
                .iter()
                .filter_map(|record| {
                    let text = element_text(one(*record, &selector)?);
                    let text = text.trim();
                    (!text.is_empty() && text.len() <= 300).then_some(text.len())
                })
                .collect();
            (texts.len() >= needed).then(|| {
                let average = texts.iter().sum::<usize>() / texts.len();
                (path, texts.len(), average)
            })
        })
        .max_by_key(|(path, hits, average)| (*hits, *average, std::cmp::Reverse(path.len())))
        .map(|(path, _, average)| (path, average))
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
                    && !ad_word(class)
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
        let matched: Vec<ElementRef<'_>> = document.select(&selector).take(40).collect();
        let Some(without) = ads_excluded(&matched) else {
            continue;
        };
        let items = format!("{items}{without}");
        let Ok(selector) = Selector::parse(&items) else {
            continue;
        };
        let records: Vec<ElementRef<'_>> = document.select(&selector).take(40).collect();
        if records.len() < 3 || records.iter().any(|record| furniture(*record)) {
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
        shared_fields(&records, &mut fields);
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
    let needed = needed(records.len());
    for &(selector, attribute) in options {
        let parsed = match selector {
            "" => None,
            _ => Some(Selector::parse(selector).ok()?),
        };
        let hits = records
            .iter()
            .filter(|record| {
                let target = match &parsed {
                    Some(selector) => one(**record, selector),
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

/// What a page that is one thing (an article, a product, a bug, a package)
/// says about it, as page-level fields: the headline, the body text, what
/// sits next to the headline, a date, an author, and the values the page
/// labels in definition lists and label-value rows.
#[derive(Clone, Debug, Default)]
pub struct Main {
    /// Selectors on the whole page, each matching exactly once. The
    /// `headline` is required; the rest may be empty on another page.
    pub fields: BTreeMap<String, OutputField>,
    /// The headline's selector, for a response check.
    pub headline: Field,
    pub body: bool,
    /// Labelled values read from definition lists and label-value rows.
    pub values: usize,
}

/// Blocks a page puts its own content in, tried in order; the body when
/// none of them is on the page exactly once.
const MAIN_BLOCKS: &[&str] = &[
    "main",
    "[role=\"main\"]",
    "article",
    "#content",
    "#main-content",
    "#main",
    ".main-content",
    ".content",
];
/// The longest text a field keeps; longer bodies are left to the page.
const MAX_BODY: usize = 16_000;

/// The main content of a page that is one thing, or `None` when the page
/// has no headline, nothing to read beside it, or (given the selector of
/// the records found on it) when those records are a quarter or more of
/// its main block: then the page is the list, and the list is its read.
pub fn main_content(html: &str, items: Option<&str>) -> Option<Main> {
    let document = Html::parse_document(html);
    let block = MAIN_BLOCKS
        .iter()
        .filter_map(|selector| Selector::parse(selector).ok())
        .find_map(|selector| {
            let mut matches = document.select(&selector);
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        })
        .unwrap_or_else(|| document.root_element());
    let mut counted = Vec::new();
    text_lengths(document.root_element(), &mut counted);
    let lengths: HashMap<_, usize> = counted.into_iter().map(|(e, n)| (e.id(), n)).collect();
    let length = |e: ElementRef<'_>| lengths.get(&e.id()).copied().unwrap_or(0);
    let block_text = length(block);
    // The lists the records sit in (their parents: a `dt` says little on
    // its own, its `dl` holds the page), for the share of the main block
    // they take and for the body text, which must sit outside them.
    let mut lists: Vec<ElementRef<'_>> = Vec::new();
    if let Some(items) = items
        && let Ok(selector) = Selector::parse(items)
    {
        for record in document.select(&selector).take(1000) {
            if let Some(parent) = record.parent().and_then(ElementRef::wrap)
                && !lists.iter().any(|p| p.id() == parent.id())
            {
                lists.push(parent);
            }
        }
    }
    let list_text: usize = lists.iter().map(|list| length(*list)).sum();
    let h1 = Selector::parse("h1").ok()?;
    // The first heading of the page itself: not the site's name in its
    // header, not a heading in a menu or a sidebar.
    let headline = block.select(&h1).find(|h| {
        !element_text(*h).trim().is_empty()
            && !h.ancestors().filter_map(ElementRef::wrap).any(|a| {
                let site_header = a.value().name() == "header"
                    && a.parent()
                        .and_then(ElementRef::wrap)
                        .is_some_and(|p| p.value().name() == "body");
                site_header
                    || matches!(a.value().name(), "nav" | "aside" | "footer")
                    || (a.value().name() != "header" && furniture_self(a))
            })
    })?;
    let headline_selector = unique_selector(&document, headline)?;
    let mut fields = BTreeMap::new();
    let text = |selector: String, attribute: &str, required: bool| OutputField {
        selector,
        attribute: attribute.to_owned(),
        required,
        absolute_url: false,
    };
    fields.insert("headline".into(), text(headline_selector.clone(), "", true));
    let mut main = Main {
        headline: Field {
            selector: headline_selector,
            attribute: String::new(),
        },
        ..Main::default()
    };

    // What the page says next to its headline, under a name the site gave
    // it: `project-header__summary` is `summary`.
    let headline_class = headline
        .value()
        .classes()
        .find(|class| selector_word(class))
        .unwrap_or_default()
        .to_owned();
    let beside: Vec<ElementRef<'_>> = headline
        .parent()
        .and_then(ElementRef::wrap)
        .map(|parent| {
            parent
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|e| e.id() != headline.id())
                .collect()
        })
        .unwrap_or_default();
    let mut beside_count = 0;
    for element in beside.into_iter().take(12) {
        if beside_count >= 5 {
            break;
        }
        let Some(class) = element.value().classes().find(|class| selector_word(class)) else {
            continue;
        };
        let value = element_text(element);
        let value = value.trim();
        if value.is_empty() || value.len() > 300 {
            continue;
        }
        let Some(name) = class_name(class, &headline_class) else {
            continue;
        };
        if fields.contains_key(&name) {
            continue;
        }
        if let Some(selector) = unique_selector(&document, element) {
            fields.insert(name, text(selector, "", false));
            beside_count += 1;
        }
    }

    let body = body_block(&document, block, headline, &lists, &length);
    // The page is its list, not one thing, when the list takes a quarter or
    // more of the main block and what is written beside it is less than a
    // quarter of the list.
    if list_text * 4 >= block_text && !body.as_ref().is_some_and(|(_, len)| len * 4 >= list_text) {
        return None;
    }
    if let Some((body, _)) = body {
        main.body = true;
        fields.insert("body".into(), text(body, "", false));
    }
    for (name, options) in [
        (
            "description",
            &[
                ("meta[name=\"description\"]", "content"),
                ("meta[property=\"og:description\"]", "content"),
            ][..],
        ),
        (
            "published_at",
            &[
                ("meta[property=\"article:published_time\"]", "content"),
                ("time[datetime]", "datetime"),
                ("time", ""),
            ][..],
        ),
        (
            "author",
            &[
                ("meta[name=\"author\"]", "content"),
                ("[rel=\"author\"]", ""),
                (".author", ""),
                (".byline", ""),
            ][..],
        ),
    ] {
        if fields.contains_key(name) {
            continue;
        }
        let found = options.iter().find_map(|(selector, attribute)| {
            let field = Field {
                selector: (*selector).to_owned(),
                attribute: (*attribute).to_owned(),
            };
            field.is_present(&document).then_some(field)
        });
        if let Some(field) = found {
            fields.insert(name.into(), text(field.selector, &field.attribute, false));
        }
    }
    main.values = labelled_values(&document, block, &mut fields);
    if !main.body && !fields.contains_key("published_at") && main.values < 2 && beside_count == 0 {
        return None;
    }
    main.fields = fields;
    Some(main)
}

/// The length of the text each element shows, in one pass: what
/// `element_text` would count, give or take the spaces between nodes,
/// without hidden parts, scripts and styles.
fn text_lengths<'a>(element: ElementRef<'a>, out: &mut Vec<(ElementRef<'a>, usize)>) -> usize {
    let value = element.value();
    let hidden = matches!(value.name(), "script" | "style" | "noscript" | "template")
        || value.attr("hidden").is_some()
        || value.attr("aria-hidden") == Some("true");
    let mut total = 0;
    if !hidden {
        for child in element.children() {
            match ElementRef::wrap(child) {
                Some(child) => total += text_lengths(child, out),
                None => {
                    if let Some(text) = child.value().as_text() {
                        let text = text.trim();
                        if !text.is_empty() {
                            total += text.len() + 1;
                        }
                    }
                }
            }
        }
    }
    out.push((element, total));
    total
}

/// Names pages give the block their text is in.
const BODY_WORDS: &[&str] = &[
    "description",
    "body",
    "content",
    "readme",
    "markdown",
    "article",
    "post",
    "entry",
    "prose",
    "text",
];

/// The block that holds the page's text, outside the lists of records: the
/// longest block named as one (`div.project-description`, `#readme`,
/// `[itemprop=articleBody]`), else down from the main block while one child
/// holds most of the text, stopping at a paragraph. Its selector and length
/// when it is long enough to be a body and short enough to keep.
fn body_block(
    document: &Html,
    block: ElementRef<'_>,
    headline: ElementRef<'_>,
    lists: &[ElementRef<'_>],
    length: &dyn Fn(ElementRef<'_>) -> usize,
) -> Option<(String, usize)> {
    let holds_list = |e: ElementRef<'_>| {
        lists.iter().any(|list| {
            list.id() == e.id()
                || list
                    .ancestors()
                    .filter_map(ElementRef::wrap)
                    .any(|a| a.id() == e.id())
        })
    };
    let fits = |e: ElementRef<'_>| {
        let length = length(e);
        ((200..=MAX_BODY).contains(&length) && e.id() != headline.id() && !holds_list(e))
            .then_some(length)
    };
    let named = block
        .descendants()
        .filter_map(ElementRef::wrap)
        .take(5000)
        .filter(|e| matches!(e.value().name(), "div" | "section" | "article" | "main"))
        .filter(|e| {
            e.value()
                .classes()
                .chain(e.value().id())
                .chain(e.value().attr("itemprop"))
                .any(|name| {
                    let lower = name.to_ascii_lowercase();
                    BODY_WORDS.iter().any(|word| lower.contains(word))
                })
        })
        .filter(|e| !furniture(*e))
        .filter_map(|e| fits(e).map(|len| (len, e)))
        .max_by_key(|(len, _)| *len);
    let found = named.or_else(|| {
        let mut node = block;
        for _ in 0..12 {
            if node.value().name() == "p" {
                break;
            }
            let total = length(node);
            let Some(child) = node
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|child| {
                    !furniture_self(*child)
                        && !matches!(child.value().name(), "script" | "style" | "nav" | "aside")
                })
                .map(|child| (length(child), child))
                .max_by_key(|(len, _)| *len)
                .filter(|(len, _)| len * 10 >= total * 3)
                .map(|(_, child)| child)
            else {
                break;
            };
            node = child;
        }
        fits(node).map(|len| (len, node))
    });
    let (length, node) = found?;
    // A body six steps down a chain of generated names is an app's layout,
    // not a block a person named; it would not match tomorrow.
    unique_selector(document, node)
        .filter(|selector| selector.matches(" > ").count() <= 6)
        .map(|selector| (selector, length))
}

/// Values the page labels: `dt` and `dd` pairs in definition lists, and
/// rows of one `th` and one `td`. Each becomes a field named after its
/// label. Returns how many were added.
fn labelled_values(
    document: &Html,
    block: ElementRef<'_>,
    fields: &mut BTreeMap<String, OutputField>,
) -> usize {
    const MAX_VALUES: usize = 12;
    let mut added = 0;
    let Ok(lists) = Selector::parse("dl") else {
        return 0;
    };
    let add = |fields: &mut BTreeMap<String, OutputField>, label: &str, selector: String| {
        let name = field_name(label);
        if name.is_empty()
            || name.len() > 40
            || !name.contains(|c: char| c.is_ascii_alphabetic())
            || secret_field(&name)
            || !stable_name(&name)
            || fields.contains_key(&name)
            || Selector::parse(&selector).is_err()
        {
            return false;
        }
        fields.insert(
            name,
            OutputField {
                selector,
                attribute: String::new(),
                required: false,
                absolute_url: false,
            },
        );
        true
    };
    for list in block.select(&lists).take(6) {
        if added >= MAX_VALUES || furniture(list) {
            break;
        }
        let children: Vec<ElementRef<'_>> = list.children().filter_map(ElementRef::wrap).collect();
        let paired = children.len() >= 2
            && children.len().is_multiple_of(2)
            && children
                .chunks(2)
                .all(|pair| pair[0].value().name() == "dt" && pair[1].value().name() == "dd");
        if !paired {
            continue;
        }
        let Some(list_selector) = unique_selector(document, list) else {
            continue;
        };
        for (index, pair) in children.chunks(2).enumerate() {
            if added >= MAX_VALUES {
                break;
            }
            let label = element_text(pair[0]);
            let value = element_text(pair[1]);
            if value.trim().is_empty() || value.len() > 2000 {
                continue;
            }
            let selector = format!("{list_selector} > dd:nth-of-type({})", index + 1);
            if add(fields, label.trim(), selector) {
                added += 1;
            }
        }
    }
    let (Ok(tables), Ok(rows), Ok(cells)) = (
        Selector::parse("table"),
        Selector::parse("tr"),
        Selector::parse("th, td"),
    ) else {
        return added;
    };
    for table in block.select(&tables).take(6) {
        if added >= MAX_VALUES || furniture(table) {
            break;
        }
        let rows: Vec<ElementRef<'_>> = table.select(&rows).take(40).collect();
        let labelled = !rows.is_empty()
            && rows.iter().all(|row| {
                let cells: Vec<ElementRef<'_>> = row.select(&cells).collect();
                cells.len() == 2
                    && cells[0].value().name() == "th"
                    && cells[1].value().name() == "td"
            });
        if !labelled {
            continue;
        }
        let Some(table_selector) = unique_selector(document, table) else {
            continue;
        };
        for (index, row) in rows.iter().enumerate() {
            if added >= MAX_VALUES {
                break;
            }
            let cells: Vec<ElementRef<'_>> = row.select(&cells).collect();
            let label = element_text(cells[0]);
            let value = element_text(cells[1]);
            if value.trim().is_empty() || value.len() > 2000 {
                continue;
            }
            let selector = format!("{table_selector} tr:nth-child({}) > td", index + 1);
            if add(fields, label.trim(), selector) {
                added += 1;
            }
        }
    }
    added
}

/// A selector that picks this element and nothing else on the page: its
/// id or its class when that is alone, else its path down from the nearest
/// ancestor with an id (or from `body`), each step placed among its
/// siblings of the same tag when it has any.
fn unique_selector(document: &Html, element: ElementRef<'_>) -> Option<String> {
    let count = |selector: &str| {
        Selector::parse(selector)
            .map(|s| document.select(&s).take(2).count())
            .unwrap_or(0)
    };
    let own: Vec<String> = element
        .value()
        .id()
        .filter(|id| selector_word(id))
        .map(|id| format!("#{id}"))
        .into_iter()
        .chain(
            element
                .value()
                .classes()
                .find(|class| selector_word(class))
                .map(|class| format!("{}.{class}", element.value().name())),
        )
        .collect();
    if let Some(found) = own.iter().find(|candidate| count(candidate) == 1) {
        return Some(found.clone());
    }
    let placed = |e: ElementRef<'_>| {
        let tag = e.value().name();
        let same = |s: &ElementRef<'_>| s.value().name() == tag;
        let before = e
            .prev_siblings()
            .filter_map(ElementRef::wrap)
            .filter(same)
            .count();
        let after = e
            .next_siblings()
            .filter_map(ElementRef::wrap)
            .filter(same)
            .count();
        let step = segment(e);
        if before + after == 0 {
            step
        } else {
            format!("{step}:nth-of-type({})", before + 1)
        }
    };
    let mut path = vec![placed(element)];
    let mut node = element.parent().and_then(ElementRef::wrap);
    while let Some(ancestor) = node {
        if matches!(ancestor.value().name(), "html" | "body") || path.len() > 12 {
            break;
        }
        match ancestor.value().id().filter(|id| selector_word(id)) {
            Some(id) => {
                path.insert(0, format!("#{id}"));
                break;
            }
            None => path.insert(0, placed(ancestor)),
        }
        node = ancestor.parent().and_then(ElementRef::wrap);
    }
    let selector = path.join(" > ");
    (selector.len() <= 500 && count(&selector) == 1).then_some(selector)
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
            let mut file = None;
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
                    "file" => {
                        file.get_or_insert_with(|| name.to_owned());
                        continue;
                    }
                    "submit" | "button" | "reset" | "image" => continue,
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
                file,
            }
        })
        .collect()
}

/// Words a page uses to label a link as an attachment, in English and
/// Korean. Only the label counts: a link to a document that the page does
/// not call an attachment is a page like any other.
const ATTACHMENT_WORDS: [&str; 7] = [
    "download",
    "attachment",
    "attached",
    "다운로드",
    "내려받기",
    "첨부",
    "내려받",
];

/// Same-origin links the page labels as attachments: a `download`
/// attribute, attachment words in the link's text, title, label or class,
/// or an icon inside it named for one. In page order, at most 200.
pub fn attachment_links(html: &str, base: &str) -> Vec<Attachment> {
    let document = Html::parse_document(html);
    let Ok(base) = url::Url::parse(base) else {
        return Vec::new();
    };
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let labelled = |text: &str| {
        let lower = text.to_lowercase();
        ATTACHMENT_WORDS.iter().any(|word| lower.contains(word))
    };
    let mut out: Vec<Attachment> = Vec::new();
    for link in document.select(&selector).take(2000) {
        let Ok(url) = base.join(link.value().attr("href").unwrap_or_default()) else {
            continue;
        };
        if url.origin() != base.origin()
            || !matches!(url.scheme(), "http" | "https")
            || url.fragment().is_some_and(|f| !f.is_empty()) && url.path() == base.path()
        {
            continue;
        }
        let text: String = link.text().collect::<Vec<_>>().join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let own = [
            link.value().attr("title"),
            link.value().attr("aria-label"),
            link.value().attr("class"),
        ]
        .into_iter()
        .flatten()
        .chain(std::iter::once(text.as_str()))
        .any(labelled);
        let icon = link
            .descendants()
            .filter_map(ElementRef::wrap)
            .any(|child| {
                ["class", "alt", "src", "title", "aria-label", "href"]
                    .iter()
                    .filter_map(|name| child.value().attr(name))
                    .any(labelled)
            });
        if link.value().attr("download").is_none() && !own && !icon {
            continue;
        }
        let mut url = url;
        url.set_fragment(None);
        if out.iter().any(|seen| seen.url == url.as_str()) {
            continue;
        }
        out.push(Attachment {
            url: url.to_string(),
            text,
        });
        if out.len() == 200 {
            break;
        }
    }
    out
}

/// A sign-in form as a person's submit would send it: where it posts to, the
/// names of its user and password fields, and the fields that go along
/// (hidden ones with their values, checked boxes, the submit button). Only
/// the sign-in itself reads this; a draft never keeps the values.
#[derive(Clone, PartialEq, Eq)]
pub struct SignInForm {
    /// Resolved against the page address.
    pub action: String,
    pub user: String,
    pub password: String,
    pub fields: Vec<(String, String)>,
}

/// Values may be tokens, so `Debug` shows names only.
impl std::fmt::Debug for SignInForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignInForm")
            .field("action", &self.action)
            .field("user", &self.user)
            .field("password", &self.password)
            .field(
                "fields",
                &self
                    .fields
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// The page's sign-in form: the first `POST` form with a password field. When
/// a definition names the fields (`user`, `password`), the form that has
/// both is taken and they are used as named; otherwise the user field is the
/// last text field before the password.
pub fn sign_in_form(
    html: &str,
    base: &str,
    user: Option<&str>,
    password: Option<&str>,
) -> Option<SignInForm> {
    let document = Html::parse_document(html);
    let base = url::Url::parse(base).ok()?;
    let form_selector = Selector::parse("form").ok()?;
    let control_selector = Selector::parse("input, button").ok()?;
    'forms: for form in document.select(&form_selector).take(40) {
        if form
            .value()
            .attr("method")
            .is_none_or(|m| !m.trim().eq_ignore_ascii_case("post"))
        {
            continue;
        }
        let mut fields = Vec::new();
        let mut last_text: Option<String> = None;
        let mut user_field: Option<String> = None;
        let mut password_field: Option<String> = None;
        let mut submit: Option<(String, String)> = None;
        for control in form.select(&control_selector).take(80) {
            let element = control.value();
            let Some(name) = element
                .attr("name")
                .map(str::trim)
                .filter(|n| !n.is_empty())
            else {
                continue;
            };
            let value = element.attr("value").unwrap_or_default();
            let kind = element
                .attr("type")
                .map(|t| t.trim().to_ascii_lowercase())
                .unwrap_or_else(|| {
                    if element.name() == "button" {
                        "submit".into()
                    } else {
                        "text".into()
                    }
                });
            match kind.as_str() {
                "password" => {
                    if password.is_some_and(|wanted| wanted != name) {
                        continue 'forms;
                    }
                    if password_field.is_none() {
                        password_field = Some(name.to_owned());
                    }
                }
                "hidden" => fields.push((name.to_owned(), value.to_owned())),
                "checkbox" | "radio" => {
                    if element.attr("checked").is_some() {
                        let value = if value.is_empty() { "on" } else { value };
                        fields.push((name.to_owned(), value.to_owned()));
                    }
                }
                "submit" => {
                    if submit.is_none() {
                        submit = Some((name.to_owned(), value.to_owned()));
                    }
                }
                "text" | "email" | "tel" | "" => {
                    if user.is_some_and(|wanted| wanted == name) {
                        user_field = Some(name.to_owned());
                    } else if password_field.is_none() {
                        last_text = Some(name.to_owned());
                    }
                }
                _ => {}
            }
        }
        let Some(password_field) = password_field else {
            continue;
        };
        let Some(user_field) = (if user.is_some() {
            user_field
        } else {
            last_text
        }) else {
            continue;
        };
        if let Some(submit) = submit {
            fields.push(submit);
        }
        let action = form.value().attr("action").unwrap_or_default().trim();
        let action = base.join(action).ok().map(|mut url| {
            url.set_fragment(None);
            url.to_string()
        })?;
        return Some(SignInForm {
            action,
            user: user_field,
            password: password_field,
            fields,
        });
    }
    None
}

/// The first link on the page to a sign-in page on the same site, for a
/// page that answers "log in to see this" instead of redirecting.
pub fn sign_in_link(html: &str, base: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let base = url::Url::parse(base).ok()?;
    let selector = Selector::parse("a[href]").ok()?;
    document
        .select(&selector)
        .take(2000)
        .filter_map(|link| {
            base.join(link.value().attr("href").unwrap_or_default())
                .ok()
        })
        .find(|url| url.origin() == base.origin() && itda_types::url::looks_like_sign_in(url))
        .map(|url| itda_types::url::sign_in_page(&url).to_string())
}

/// The query key and value the page's own links carry a session in, when
/// they do: the pair whose value looks like a session on at least two
/// same-origin links. A site that keeps its session in the address puts it
/// on every link it draws. Only the sign-in reads this; a draft never keeps
/// the value.
pub fn session_in_links(html: &str, base: &str) -> Option<(String, String)> {
    let document = Html::parse_document(html);
    let base = url::Url::parse(base).ok()?;
    let selector = Selector::parse("a[href]").ok()?;
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for link in document.select(&selector).take(2000) {
        let Ok(url) = base.join(link.value().attr("href").unwrap_or_default()) else {
            continue;
        };
        if url.origin() != base.origin() {
            continue;
        }
        for (key, value) in url.query_pairs() {
            if itda_types::url::session_value(&value) {
                *counts
                    .entry((key.into_owned(), value.into_owned()))
                    .or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .max_by_key(|(_, n)| *n)
        .map(|(pair, _)| pair)
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

/// A frame the page shows another page in: an `<iframe src>` or a
/// `<frame src>` of an older frameset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Resolved against the page address, without its fragment.
    pub url: String,
    /// The frame's page is on the page's own origin (scheme, host and
    /// port), so it can be read as a page of the same site.
    pub same_origin: bool,
}

/// At most this many frames are listed: a portal has a few, not a wall.
const FRAMES: usize = 8;

/// The frames the page shows, in document order, each address once. Blank
/// and script frames (`about:blank`, `javascript:`, `data:`) are left out,
/// as is the page itself.
pub fn frames(html: &str, base: &str) -> Vec<Frame> {
    let document = Html::parse_document(html);
    let Ok(base) = url::Url::parse(base) else {
        return Vec::new();
    };
    let Ok(selector) = Selector::parse("iframe[src], frame[src]") else {
        return Vec::new();
    };
    let mut own = base.clone();
    own.set_fragment(None);
    let mut out: Vec<Frame> = Vec::new();
    for frame in document.select(&selector) {
        let Some(src) = frame.value().attr("src").map(str::trim) else {
            continue;
        };
        let Ok(mut url) = base.join(src) else {
            continue;
        };
        url.set_fragment(None);
        if !matches!(url.scheme(), "http" | "https") || url == own {
            continue;
        }
        let same_origin = url.origin() == base.origin();
        let url = url.to_string();
        if out.iter().any(|f| f.url == url) {
            continue;
        }
        out.push(Frame { url, same_origin });
        if out.len() == FRAMES {
            break;
        }
    }
    out
}

/// How a page turns its pages: the query key its links use, and the value
/// that means this page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pagination {
    pub key: String,
    /// `1` for a page number, `0` for an offset.
    pub default: i64,
}

/// The query key the page's own links use to turn pages (`?page=2`), when
/// they point at the same path: the key of the link marked as the next
/// page, else the key the most links use.
pub fn pagination(html: &str, page: &str) -> Option<Pagination> {
    let document = Html::parse_document(html);
    let page = url::Url::parse(page).ok()?;
    let selector = Selector::parse("a[href], link[href]").ok()?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut next: Option<String> = None;
    for link in document.select(&selector).take(2000) {
        let Ok(url) = page.join(link.value().attr("href").unwrap_or_default()) else {
            continue;
        };
        if url.origin() != page.origin() || url.path() != page.path() {
            continue;
        }
        let marked_next = link.value().attr("rel").is_some_and(|rel| {
            rel.split_whitespace()
                .any(|r| r.eq_ignore_ascii_case("next"))
        });
        for (key, value) in url.query_pairs() {
            if value.parse::<u64>().is_err() {
                continue;
            }
            let known = PAGE_KEYS.contains(&key.to_ascii_lowercase().as_str());
            if marked_next && next.is_none() && (known || url.query_pairs().count() == 1) {
                next = Some(key.to_string());
            }
            if known {
                *counts.entry(key.into_owned()).or_default() += 1;
            }
        }
    }
    let key = next.or_else(|| {
        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(key, _)| key)
    })?;
    let default = if matches!(key.to_ascii_lowercase().as_str(), "offset" | "start") {
        0
    } else {
        1
    };
    Some(Pagination { key, default })
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

/// What a page says about the software that made it: its `<meta>` tags,
/// `<link>` tags, script addresses and body classes, as written, so a
/// caller can recognise GitLab, WordPress or Jira without parsing the page
/// again. Bounded: at most 500 of each.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Markers {
    /// `(name or property, content)` of every `<meta>` with both.
    pub metas: Vec<(String, String)>,
    /// `(rel, href)` of every `<link>` with both.
    pub links: Vec<(String, String)>,
    /// `src` of every `<script src>`.
    pub scripts: Vec<String>,
    /// The `<body>`'s `class` attribute, or `""`.
    pub body_class: String,
    /// The text of every inline `<script>`, for settings pages embed such as
    /// Gitea's `window.config`; each cut at 4 KB.
    pub inline_scripts: Vec<String>,
}

impl Markers {
    /// The content of the first `<meta name=...>` (or `property=...`) with
    /// that name, case-insensitively.
    pub fn meta(&self, name: &str) -> Option<&str> {
        self.metas
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, content)| content.as_str())
    }

    /// The `href` of the first `<link>` whose `rel` list contains `rel`.
    pub fn link(&self, rel: &str) -> Option<&str> {
        self.links
            .iter()
            .find(|(r, _)| {
                r.split_ascii_whitespace()
                    .any(|part| part.eq_ignore_ascii_case(rel))
            })
            .map(|(_, href)| href.as_str())
    }

    /// Whether the body carries `class`.
    pub fn body_has_class(&self, class: &str) -> bool {
        self.body_class
            .split_ascii_whitespace()
            .any(|c| c.eq_ignore_ascii_case(class))
    }
}

pub fn markers(html: &str) -> Markers {
    let document = Html::parse_document(html);
    let mut out = Markers::default();
    if let Ok(selector) = Selector::parse("meta") {
        for meta in document.select(&selector).take(500) {
            let element = meta.value();
            let name = element.attr("name").or_else(|| element.attr("property"));
            if let (Some(name), Some(content)) = (name, element.attr("content")) {
                out.metas
                    .push((name.trim().to_owned(), content.trim().to_owned()));
            }
        }
    }
    if let Ok(selector) = Selector::parse("link[rel][href]") {
        for link in document.select(&selector).take(500) {
            let element = link.value();
            if let (Some(rel), Some(href)) = (element.attr("rel"), element.attr("href")) {
                out.links
                    .push((rel.trim().to_owned(), href.trim().to_owned()));
            }
        }
    }
    if let Ok(selector) = Selector::parse("script") {
        for script in document.select(&selector).take(500) {
            match script.value().attr("src") {
                Some(src) => out.scripts.push(src.trim().to_owned()),
                None => {
                    let text: String = script.text().collect();
                    if !text.trim().is_empty() {
                        out.inline_scripts.push(text.chars().take(4096).collect());
                    }
                }
            }
        }
    }
    if let Ok(selector) = Selector::parse("body")
        && let Some(body) = document.select(&selector).next()
    {
        out.body_class = body.value().attr("class").unwrap_or_default().to_owned();
    }
    out
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

/// True when the page could fetch something by itself once a browser runs
/// it: it loads a script file (a `script src`, or a module it preloads), or
/// an inline script mentions `fetch`, `XMLHttpRequest`, `graphql`, `api` or
/// imports a module. Embedded JSON does not count. A page without any of
/// that has shown a plain fetch all it has.
pub fn fetches_by_itself(html: &str) -> bool {
    let document = Html::parse_document(html);
    let preloads = Selector::parse("link[rel=modulepreload]")
        .map(|s| document.select(&s).count())
        .unwrap_or(0);
    if preloads > 0 {
        return true;
    }
    let Ok(selector) = Selector::parse("script") else {
        return false;
    };
    document.select(&selector).any(|script| {
        let kind = script
            .value()
            .attr("type")
            .unwrap_or("")
            .to_ascii_lowercase();
        if kind.contains("json") || kind == "text/template" {
            return false;
        }
        if script.value().attr("src").is_some() {
            return true;
        }
        let text = script.text().collect::<String>().to_ascii_lowercase();
        ["fetch", "xmlhttprequest", "graphql", "api", "import("]
            .iter()
            .any(|word| text.contains(word))
    })
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
    fn ad_slots_beside_the_records_are_left_out() {
        let titles = |html: &str| -> (String, Vec<String>) {
            let extraction = infer_extraction(html, BASE).unwrap();
            let value = extraction.extract(html, BASE).unwrap();
            let titles = value["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["title"].as_str().unwrap().to_owned())
                .collect();
            (extraction.items, titles)
        };
        let books = ["Book 1", "Book 2", "Book 3", "Book 4"];
        // Results that share their classes with the ads between them, the ads
        // marked by one more class that sorts first.
        let row = |class: &str, title: &str| {
            format!("<div class='{class}'><h2><a href='/dp/{title}'>{title}</a></h2></div>")
        };
        let mut slots = String::new();
        for (n, book) in books.iter().enumerate() {
            slots += &row("sg-col gsx-anchor AdHolder", &format!("Sponsored pick {n}"));
            slots += &row("sg-col gsx-anchor", book);
        }
        let html = format!("<main><div class='slot'>{slots}</div></main>");
        let (items, found) = titles(&html);
        assert!(items.ends_with(":not(.AdHolder)"), "{items}");
        assert_eq!(found, books);

        let cards = html.replace("sg-col gsx-anchor", "result-card");
        let (items, found) = titles(&cards);
        assert_eq!(items, "div.result-card:not(.AdHolder)");
        assert_eq!(found, books);

        for word in ["AdHolder", "ad-banner", "ads", "sponsored-post"] {
            assert!(ad_word(word), "{word}");
        }
        for word in ["add-to-cart", "header", "badge", "load-more", "lead"] {
            assert!(!ad_word(word), "{word}");
        }
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
    fn unnamed_lists_are_found_by_their_repeated_link_and_navigation_is_skipped() {
        let stories: String = (1..=4)
            .map(|n| format!("<li class='story'><div class='story_liner'><span class='link'><a class='u-url' href='/s/{n}'>Story {n} title</a></span><span class='tags'><a href='/t/a'>a</a> <a href='/t/b'>b</a></span><span class='byline'>by someone</span></div></li>"))
            .collect();
        let html = format!(
            "<nav><ol>{}</ol></nav><main><h1>Front</h1><ol class='stories list'>{stories}</ol></main>",
            (1..=6)
                .map(|n| format!("<li><a href='/nav/{n}'>Menu {n}</a></li>"))
                .collect::<String>()
        );
        let extraction = infer_extraction(&html, BASE).unwrap();
        assert!(
            extraction.items.starts_with("ol.") && extraction.items.ends_with(" > li.story"),
            "{}",
            extraction.items
        );
        assert_eq!(
            extraction.fields["title"].selector,
            "div.story_liner > span.link > a.u-url"
        );
        assert_eq!(extraction.fields["link"].attribute, "href");
        let value = extraction.extract(&html, BASE).unwrap();
        assert_eq!(value["items"][2]["title"], "Story 3 title");
        assert_eq!(value["items"][2]["link"], "https://example.test/s/3");

        let listing: String = (1..=5)
            .map(|n| format!("<dt><a href='/abs/{n}'>arXiv:{n}</a></dt><dd><div class='list-title'>Paper {n}</div></dd>"))
            .collect();
        let html = format!(
            "<nav><ul><li><a href='/x'>x</a></li><li><a href='/y'>y</a></li><li><a href='/z'>z</a></li></ul></nav><dl id='articles'>{listing}</dl>"
        );
        let extraction = infer_extraction(&html, BASE).unwrap();
        assert_eq!(extraction.items, "#articles > dt");
        assert_eq!(extraction.fields["title"].selector, "a");
    }

    #[test]
    fn anchors_with_named_parts_are_records_and_bare_links_or_paragraphs_are_not() {
        let releases: String = (1..=4)
            .map(|n| format!("<a class='release' href='/p/1.{n}/'><p class='release__version'>1.{n}</p><p class='release__meta'><time datetime='2026-01-0{n}'>Jan {n}</time></p></a>"))
            .collect();
        let html = format!("<main><h1>Pkg</h1><div class='timeline'>{releases}</div></main>");
        let extraction = infer_extraction(&html, BASE).unwrap();
        assert_eq!(extraction.items, "div.timeline > a.release");
        assert_eq!(extraction.fields["title"].selector, "");
        assert_eq!(extraction.fields["link"].attribute, "href");
        assert_eq!(extraction.fields["version"].selector, "p.release__version");
        assert_eq!(extraction.fields["published_at"].attribute, "datetime");
        let value = extraction.extract(&html, BASE).unwrap();
        assert_eq!(value["items"][2]["version"], "1.3");
        assert_eq!(value["items"][2]["link"], "https://example.test/p/1.3/");

        let tags = "<main><h1>Tags</h1><div class='cloud'><a class='tag' href='/t/a'>a</a><a class='tag' href='/t/b'>b</a><a class='tag' href='/t/c'>c</a><a class='tag' href='/t/d'>d</a></div></main>";
        assert!(infer_extraction(tags, BASE).is_none());
        let prose = format!(
            "<main><h1>Essay</h1><div class='text'>{}</div></main>",
            (1..=4)
                .map(|n| format!("<p>Paragraph {n} says something and <a href='/ref/{n}'>cites</a> a source at length.</p>"))
                .collect::<String>()
        );
        assert!(infer_extraction(&prose, BASE).is_none());
        let built = "<ul class='l__9fQ1'><li class='i__X41oTe'><h3><a href='/1'>One</a></h3><span class='badge__4AEjpg'>x</span></li><li class='i__X41oTe'><h3><a href='/2'>Two</a></h3><span class='badge__4AEjpg'>y</span></li><li class='i__X41oTe'><h3><a href='/3'>Three</a></h3><span class='badge__4AEjpg'>z</span></li></ul>";
        let extraction = infer_extraction(built, BASE).unwrap();
        assert!(
            extraction
                .fields
                .keys()
                .all(|k| ["title", "link"].contains(&k.as_str())),
            "{:?}",
            extraction.fields.keys()
        );
        assert!(date_like("2026-01-05") && date_like("Jan 5, 2026") && date_like("3 hours ago"));
        assert!(date_like("2026년 1월 5일") && date_like("2일 전") && date_like("05/01/2026"));
        assert!(!date_like("120") && !date_like("v2.34.2") && !date_like("Blue bicycle"));
        assert_eq!(
            class_name("post-card__author", "post-card").as_deref(),
            Some("author")
        );
        assert_eq!(
            class_name("post-byline", "post-title").as_deref(),
            Some("byline")
        );
        assert_eq!(class_name("heading__fzQskI", ""), None);
        assert_eq!(class_name("container", ""), None);
        assert_eq!(class_name("csrf-token", ""), None);
    }

    #[test]
    fn main_content_reads_one_thing_and_leaves_a_list_page_alone() {
        let body = "<p>".to_owned() + &"A sentence of the body text. ".repeat(12) + "</p>";
        let html = format!(
            "<header><h1>Site</h1><nav><a href='/'>Home</a></nav></header><main><article><h1 class='title'>One thing</h1><p class='byline'>by someone</p><div class='content'>{body}</div><dl><dt>Status</dt><dd>Open</dd><dt>Owner</dt><dd>Mina</dd></dl></article></main>"
        );
        let main = main_content(&html, None).unwrap();
        assert_eq!(main.headline.selector, "h1.title");
        assert!(main.fields["headline"].required);
        assert_eq!(main.fields["body"].selector, "div.content");
        assert_eq!(main.fields["byline"].selector, "p.byline");
        assert_eq!(
            main.fields["status"].selector,
            "main > article > dl > dd:nth-of-type(1)"
        );
        assert_eq!(
            main.fields["owner"].selector,
            "main > article > dl > dd:nth-of-type(2)"
        );
        assert_eq!(main.values, 2);
        assert!(main.body);

        let stories: String = (1..=6)
            .map(|n| format!("<li class='story'><a href='/s/{n}'>Story number {n} with a long enough title</a></li>"))
            .collect();
        let list = format!("<main><h1>Front</h1><ol class='stories'>{stories}</ol></main>");
        assert!(main_content(&list, Some("ol.stories > li.story")).is_none());
        assert!(main_content("<title>Only</title><p>short</p>", None).is_none());
        assert!(main_content("<h1>Head</h1><p>short</p>", None).is_none());
    }

    #[test]
    fn pagination_prefers_the_next_link_and_offsets_start_at_zero() {
        let html = "<a href='/board?p=2'>2</a><a href='/board?p=3'>3</a><a rel='next' href='/board?start=30'>next</a>";
        assert_eq!(
            pagination(html, BASE),
            Some(Pagination {
                key: "start".into(),
                default: 0
            })
        );
        let html = "<a href='/board?offset=20'>more</a><a href='/board?offset=40'>x</a>";
        assert_eq!(pagination(html, BASE).unwrap().default, 0);
        assert!(pagination("<a href='/other?page=2'>x</a>", BASE).is_none());
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
        assert_eq!(forms[1].file.as_deref(), Some("upload"));
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
    fn only_links_the_page_calls_attachments_are_attachments() {
        let html = r#"<a href="/files/1">Report</a>
            <a href="/files/2" download>Report</a>
            <a href="/files/3"><img class="icon-download" src="/dl.png"></a>
            <a href="/board/files/4.pdf" title="첨부파일 다운로드">보고서.pdf</a>
            <a href="/files/5" class="attachment">plan</a>
            <a href="https://other.test/files/6" download>elsewhere</a>
            <a href="/files/2#top" download>again</a>"#;
        let links = attachment_links(html, BASE);
        let urls: Vec<&str> = links.iter().map(|a| a.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://example.test/files/2",
                "https://example.test/files/3",
                "https://example.test/board/files/4.pdf",
                "https://example.test/files/5",
            ]
        );
        assert_eq!(links[2].text, "보고서.pdf");
    }

    #[test]
    fn the_sign_in_form_is_read_as_a_person_would_submit_it() {
        let html = r#"<form action="/search" method="get"><input name="q"></form>
            <form action="/session?lang=en" method="post">
              <input type="hidden" name="_csrf" value="tok-1">
              <input type="hidden" name="next" value="/home">
              <input type="email" name="email" required>
              <input type="password" name="pass">
              <input type="checkbox" name="remember" value="1">
              <input type="checkbox" name="agree" value="yes" checked>
              <button type="submit" name="commit" value="Sign in">Sign in</button>
            </form>"#;
        let form = sign_in_form(html, BASE, None, None).unwrap();
        assert_eq!(form.action, "https://example.test/session?lang=en");
        assert_eq!(form.user, "email");
        assert_eq!(form.password, "pass");
        assert_eq!(
            form.fields,
            [
                ("_csrf".to_owned(), "tok-1".to_owned()),
                ("next".to_owned(), "/home".to_owned()),
                ("agree".to_owned(), "yes".to_owned()),
                ("commit".to_owned(), "Sign in".to_owned()),
            ]
        );
        assert!(!format!("{form:?}").contains("tok-1"));

        // The fields a definition names are used as named; other names are
        // not that form.
        assert_eq!(
            sign_in_form(html, BASE, Some("email"), Some("pass"))
                .unwrap()
                .user,
            "email"
        );
        assert!(sign_in_form(html, BASE, Some("login"), Some("pass")).is_none());
        // A GET form never carries a password anywhere.
        assert!(
            sign_in_form(
                "<form action='/x'><input name='u'><input type='password' name='p'></form>",
                BASE,
                None,
                None
            )
            .is_none()
        );
    }

    #[test]
    fn the_link_to_the_sign_in_page_is_found() {
        let html = r#"<a href="/wiki/Main">Main</a>
            <a href="https://other.test/login">elsewhere</a>
            <a href="/index.php?title=Special:UserLogin&returnto=Special%3AAllPages#top">Log in</a>"#;
        assert_eq!(
            sign_in_link(html, BASE).as_deref(),
            Some("https://example.test/index.php?title=Special%3AUserLogin")
        );
        assert_eq!(sign_in_link("<a href='/about'>x</a>", BASE), None);
    }

    #[test]
    fn feeds_pagination_and_titles_are_found() {
        let html = r#"<head><title>Board</title><link rel="alternate" type="application/rss+xml" title="Posts" href="/feed.xml"><link rel="service-desc" href="/openapi.json"></head>
            <body><h1>Notices</h1><a href="/board?page=2">2</a><a href="/board?page=3">3</a><a href="/other?page=9">x</a></body>"#;
        assert_eq!(feeds(html, BASE)[0].url, "https://example.test/feed.xml");
        assert_eq!(feeds(html, BASE)[0].title, "Posts");
        assert_eq!(pagination(html, BASE).unwrap().key, "page");
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

    #[test]
    fn markers_keep_metas_links_scripts_and_body_classes() {
        let html = "<html><head><meta name='generator' content='WordPress 6.5'>
            <meta property='og:site_name' content='GitLab'>
            <link rel='https://api.w.org/' href='https://blog.example/wp-json/'>
            <link rel='EditURI alternate' type='application/rsd+xml' href='//wiki.example/w/api.php?action=rsd'>
            <script src='/assets/webpack/main.js'></script><script>window.config={appSubUrl:''}</script>
            </head><body class='skin-vector mediawiki'></body></html>";
        let found = markers(html);
        assert_eq!(found.meta("Generator"), Some("WordPress 6.5"));
        assert_eq!(found.meta("og:site_name"), Some("GitLab"));
        assert_eq!(
            found.link("https://api.w.org/"),
            Some("https://blog.example/wp-json/")
        );
        assert_eq!(
            found.link("edituri"),
            Some("//wiki.example/w/api.php?action=rsd")
        );
        assert_eq!(found.scripts, vec!["/assets/webpack/main.js".to_owned()]);
        assert!(found.inline_scripts[0].contains("appSubUrl"));
        assert!(found.body_has_class("mediawiki"));
        assert!(!found.body_has_class("skin"));
    }

    #[test]
    fn frames_are_listed_once_with_their_origin_and_blank_ones_left_out() {
        let html = r#"<title>Portal</title>
            <iframe src="/menu.do#top"></iframe>
            <iframe src="board/list.do?bbs=1"></iframe>
            <iframe src="https://cdn.example.net/widget"></iframe>
            <iframe src="/menu.do"></iframe>
            <iframe src="about:blank"></iframe>
            <iframe src="javascript:void(0)"></iframe>
            <iframe src="data:text/html,hi"></iframe>
            <iframe src="/portal/"></iframe>
            <iframe></iframe>"#;
        let found = frames(html, "https://intranet.example.co.kr/portal/");
        let listed: Vec<(&str, bool)> = found
            .iter()
            .map(|f| (f.url.as_str(), f.same_origin))
            .collect();
        assert_eq!(
            listed,
            [
                ("https://intranet.example.co.kr/menu.do", true),
                (
                    "https://intranet.example.co.kr/portal/board/list.do?bbs=1",
                    true
                ),
                ("https://cdn.example.net/widget", false),
            ]
        );
        assert!(
            !frames(
                "<iframe src='http://intranet.example.co.kr:8080/menu.do'></iframe>",
                "http://intranet.example.co.kr/portal/"
            )[0]
            .same_origin,
            "another port is another origin"
        );
        // An older frameset counts the same way.
        let frameset = r#"<html><head><title>Main</title></head>
            <frameset cols="20%,80%"><frame src="/menu.do" name="menu"><frame src="/board.do"></frameset></html>"#;
        let found = frames(frameset, "https://intranet.example.co.kr/main.do");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[1].url, "https://intranet.example.co.kr/board.do");
        assert!(frames("<h1>No frames</h1>", "https://a.test/").is_empty());
    }

    #[test]
    fn the_session_key_links_carry_is_named_only_when_links_agree() {
        let html = r#"<a href="/notices?sid=a1b2c3d4e5f6&page=2">next</a>
            <a href="/notices/1?sid=a1b2c3d4e5f6">one</a>
            <a href="/help?topic=faq">help</a>
            <a href="https://other.example/x?sid=a1b2c3d4e5f6">elsewhere</a>"#;
        assert_eq!(
            session_in_links(html, "https://a.test/notices"),
            Some(("sid".to_owned(), "a1b2c3d4e5f6".to_owned()))
        );
        assert_eq!(
            session_in_links("<a href='/n?sid=a1b2c3d4e5f6'>once</a>", "https://a.test/"),
            None
        );
        assert_eq!(
            session_in_links(
                "<a href='/n?page=2'>a</a><a href='/n?page=3'>b</a>",
                "https://a.test/"
            ),
            None
        );
    }

    #[test]
    fn a_page_fetches_by_itself_only_through_scripts() {
        assert!(fetches_by_itself(
            "<div id=root></div><script src=/app.js></script>"
        ));
        assert!(fetches_by_itself(
            "<h1>Board</h1><script>fetch('/api/items').then(r => r.json())</script>"
        ));
        assert!(fetches_by_itself(
            "<h1>Board</h1><script>var x = new XMLHttpRequest()</script>"
        ));
        assert!(!fetches_by_itself(
            "<h1>Board</h1><p>Text only.</p><script type='application/ld+json'>{\"url\": \"https://a.example/api\"}</script>"
        ));
        assert!(fetches_by_itself(
            "<div></div><script>import('/_app/start.js').then(k => k.init())</script>"
        ));
        assert!(fetches_by_itself(
            "<link href='/_app/entry.js' rel='modulepreload'><div></div>"
        ));
        assert!(!fetches_by_itself(
            "<h1>Board</h1><script>document.title = 'Board'</script>"
        ));
        assert!(!fetches_by_itself(
            "<h1>Board</h1><p>Nothing runs here.</p>"
        ));
    }
}
