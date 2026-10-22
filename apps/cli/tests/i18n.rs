//! The landing page and the README say the same thing in every language:
//! each `site/i18n/<tag>.js` has the keys `site/index.html` marks and keeps
//! their HTML, and each `README.<tag>.md` keeps the shape of `README.md`.
//! `docs/i18n.md` lists the languages; the CLI strings are checked in
//! `apps/cli/src/text.rs`.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const LANGS: [&str; 12] = [
    "en", "ko", "ja", "zh-Hans", "zh-Hant", "es", "fr", "de", "pt-BR", "vi", "id", "ru",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The keys `data-i18n` names in index.html, each with its English inner HTML.
fn landing_english() -> BTreeMap<String, String> {
    let html = read(&repo().join("site").join("index.html"));
    let mut out = BTreeMap::new();
    let mut rest = html.as_str();
    while let Some(at) = rest.find("data-i18n=\"") {
        let key_start = at + "data-i18n=\"".len();
        let key_end = key_start + rest[key_start..].find('"').expect("data-i18n closes");
        let key = rest[key_start..key_end].to_owned();
        let open_end = key_end + rest[key_end..].find('>').expect("open tag closes") + 1;
        let tag_start = rest[..at].rfind('<').expect("data-i18n sits in a tag");
        let tag: String = rest[tag_start + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let inner = inner_html(&rest[open_end..], &tag);
        if let Some(seen) = out.get(&key) {
            assert_eq!(
                seen, &inner,
                "index.html: {key} appears twice with different text"
            );
        }
        out.insert(key, inner);
        rest = &rest[open_end..];
    }
    out
}

/// The inner HTML of an element whose open tag just ended, up to the matching
/// close tag, nested same-name elements included.
fn inner_html(after_open: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 0usize;
    let mut i = 0;
    while i < after_open.len() {
        let rest = &after_open[i..];
        if rest.starts_with(&close) {
            if depth == 0 {
                return after_open[..i].to_owned();
            }
            depth -= 1;
            i += close.len();
        } else if rest.starts_with(&open)
            && rest[open.len()..]
                .chars()
                .next()
                .is_some_and(|c| c == '>' || c.is_whitespace())
        {
            depth += 1;
            i += open.len();
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    panic!("no </{tag}>");
}

/// The dictionary a `site/i18n/<tag>.js` file registers.
fn landing(tag: &str) -> BTreeMap<String, String> {
    let js = read(&repo().join("site").join("i18n").join(format!("{tag}.js")));
    let marker = format!("window.ITDA_I18N[\"{tag}\"] = ");
    let start = js
        .find(&marker)
        .unwrap_or_else(|| panic!("site/i18n/{tag}.js does not set window.ITDA_I18N[\"{tag}\"]"))
        + marker.len();
    let end = js
        .rfind("};")
        .unwrap_or_else(|| panic!("site/i18n/{tag}.js has no closing }};"));
    serde_json::from_str(&js[start..=end])
        .unwrap_or_else(|e| panic!("site/i18n/{tag}.js is not one JSON object: {e}"))
}

/// Every `<...>` in a string, as written, in order. Line breaks are left
/// out: where a heading breaks is each language's own choice.
fn tags(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        let end = at + rest[at..].find('>').unwrap_or(rest.len() - at - 1) + 1;
        if &rest[at..end] != "<br>" {
            out.push(rest[at..end].to_owned());
        }
        rest = &rest[end..];
    }
    out
}

/// Every address a string links to or embeds.
fn urls(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for marker in ["href=\"", "src=\"", "]("] {
        let mut rest = text;
        while let Some(at) = rest.find(marker) {
            let start = at + marker.len();
            let end = start
                + rest[start..]
                    .find(|c: char| c == '"' || c == ')' || c.is_whitespace())
                    .unwrap_or(rest.len() - start);
            out.insert(rest[start..end].to_owned());
            rest = &rest[end..];
        }
    }
    out
}

/// GitHub's anchor for a heading: lower case, letters, digits and spaces
/// only, spaces as hyphens.
fn anchor(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

fn translated_langs() -> impl Iterator<Item = &'static str> {
    LANGS.iter().copied().filter(|tag| *tag != "en")
}

#[test]
fn every_landing_language_has_every_key_and_the_same_html() {
    let english = landing_english();
    assert!(english.len() > 100, "index.html lost its data-i18n marks");
    for tag in translated_langs() {
        let own = landing(tag);
        let mut wanted: BTreeSet<&str> = english.keys().map(String::as_str).collect();
        wanted.extend(["meta.title", "meta.description", "flow.alt"]);
        let mut got: BTreeSet<&str> = own.keys().map(String::as_str).collect();
        got.remove("flow.src");
        let missing: Vec<_> = wanted.difference(&got).collect();
        let extra: Vec<_> = got.difference(&wanted).collect();
        assert!(missing.is_empty(), "site/i18n/{tag}.js lacks {missing:?}");
        assert!(
            extra.is_empty(),
            "site/i18n/{tag}.js has keys index.html lacks: {extra:?}"
        );
        for (key, en) in &english {
            let text = &own[key];
            assert!(
                !text.trim().is_empty(),
                "site/i18n/{tag}.js: {key} is empty"
            );
            let mut en_tags = tags(en);
            let mut own_tags = tags(text);
            en_tags.sort();
            own_tags.sort();
            if key == "scope.more" {
                // The one link that points at the language's own README.
                let readme = format!("README.{tag}.md#");
                assert!(
                    text.contains(&readme),
                    "site/i18n/{tag}.js: scope.more must link to {readme}..."
                );
                assert_eq!(
                    en_tags.len(),
                    own_tags.len(),
                    "site/i18n/{tag}.js: {key} tags"
                );
                continue;
            }
            assert_eq!(
                own_tags, en_tags,
                "site/i18n/{tag}.js: {key} changes the HTML"
            );
            assert_eq!(
                urls(text),
                urls(en),
                "site/i18n/{tag}.js: {key} changes a link"
            );
        }
    }
}

#[test]
fn landing_links_to_the_right_readme_heading() {
    for tag in translated_langs() {
        let own = landing(tag);
        let readme = read(&repo().join(format!("README.{tag}.md")));
        let marker = format!("README.{tag}.md#");
        let text = &own["scope.more"];
        let start = text.find(&marker).unwrap() + marker.len();
        let end = start + text[start..].find('"').unwrap();
        let linked = &text[start..end];
        let headings: Vec<String> = readme
            .lines()
            .filter_map(|l| l.strip_prefix("## "))
            .map(anchor)
            .collect();
        assert!(
            headings.iter().any(|h| h == linked),
            "site/i18n/{tag}.js links README.{tag}.md#{linked}, which has {headings:?}"
        );
    }
}

#[test]
fn every_readme_keeps_the_shape_of_the_english_one() {
    let english = read(&repo().join("README.md"));
    let headings = english.lines().filter(|l| l.starts_with("## ")).count();
    let fences = english.lines().filter(|l| l.starts_with("```")).count();
    let language_line = english
        .lines()
        .find(|l| l.contains("README.") && l.contains("Deutsch") && l.contains("Русский"))
        .expect("README.md has the language line");
    // The language line is checked on its own; the READMEs it links differ.
    let readmes = |set: BTreeSet<String>| -> BTreeSet<String> {
        set.into_iter()
            .filter(|u| !(u.starts_with("README.") && u.ends_with(".md")))
            .collect()
    };
    let en_urls = readmes(urls(&english));
    for tag in translated_langs() {
        let own = read(&repo().join(format!("README.{tag}.md")));
        assert_eq!(
            own.lines().filter(|l| l.starts_with("## ")).count(),
            headings,
            "README.{tag}.md: headings"
        );
        assert_eq!(
            own.lines().filter(|l| l.starts_with("```")).count(),
            fences,
            "README.{tag}.md: code blocks"
        );
        let own_line = own
            .lines()
            .find(|l| l.contains("README.") && l.contains("Deutsch") && l.contains("Русский"))
            .unwrap_or_else(|| panic!("README.{tag}.md has no language line"));
        // The same line, with its own language in bold instead of a link.
        let bold_start = own_line
            .find("<b>")
            .unwrap_or_else(|| panic!("README.{tag}.md: no bold language"));
        let bold_end = own_line.find("</b>").unwrap() + "</b>".len();
        let name = &own_line[bold_start + 3..bold_end - 4];
        let as_link = format!("<a href=\"README.{tag}.md\">{name}</a>");
        let canonical =
            language_line.replace("<b>English</b>", "<a href=\"README.md\">English</a>");
        let own_as_links = own_line.replace(&own_line[bold_start..bold_end], &as_link);
        assert_eq!(own_as_links, canonical, "README.{tag}.md: language line");
        // Korean was edited by hand and has its own pictures; the ten
        // translations use the English ones and link what English links.
        if tag == "ko" {
            continue;
        }
        let own_urls = readmes(urls(&own));
        let missing: Vec<_> = en_urls.difference(&own_urls).collect();
        assert!(
            missing.is_empty(),
            "README.{tag}.md lacks links {missing:?}"
        );
        let extra: Vec<_> = own_urls.difference(&en_urls).collect();
        assert!(extra.is_empty(), "README.{tag}.md adds links {extra:?}");
    }
}

#[test]
fn the_picker_lists_every_language() {
    let html = read(&repo().join("site").join("index.html"));
    let main = read(&repo().join("site").join("main.js"));
    for tag in LANGS {
        assert!(
            html.contains(&format!("<option value=\"{tag}\"")),
            "index.html picker lacks {tag}"
        );
        assert!(
            main.contains(&format!("\"{tag}\"")),
            "main.js LANGS lacks {tag}"
        );
    }
}
