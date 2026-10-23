//! What a page and the scripts it loads say about a site's routes, for an
//! agent drafting a definition: the page's headings, the paths it links to
//! on its own origin, the `GET /users/{id}` routes its prose names, the
//! scripts it loads and the route strings inside them. Paths, methods and
//! names only: never a record, a form value or a query.
use crate::element_text;
use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Headings listed per page.
pub const HEADINGS: usize = 40;
/// Characters kept of one heading.
pub const HEADING_CHARS: usize = 120;
/// Same-origin paths listed per page.
pub const PATHS: usize = 120;
/// Routes taken from one page or one script.
pub const ROUTES: usize = 500;
/// Scripts listed per page, best first.
pub const SCRIPTS: usize = 24;
/// The most text of a page or a script read for routes, in bytes.
const TEXT_LIMIT: usize = 512 * 1024;

/// One heading, `h1` to `h3`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

/// A route a page or a script names: the method and the path with its
/// variables as `{name}`, and the origin when it was written in full.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Route {
    pub method: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// The page's `h1`, `h2` and `h3`, in order, at most [`HEADINGS`], each cut
/// to [`HEADING_CHARS`].
pub fn headings(html: &str) -> Vec<Heading> {
    let document = Html::parse_document(html);
    let Ok(selector) = Selector::parse("h1, h2, h3") else {
        return Vec::new();
    };
    document
        .select(&selector)
        .filter_map(|element| {
            let level = element.value().name()[1..].parse().ok()?;
            let text: String = element_text(element).chars().take(HEADING_CHARS).collect();
            (!text.is_empty()).then_some(Heading { level, text })
        })
        .take(HEADINGS)
        .collect()
}

/// A path that is a file a browser loads for the page, not a page.
fn asset(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".css", ".js", ".mjs", ".map", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico",
        ".woff", ".woff2", ".ttf", ".otf", ".mp4", ".webm", ".mp3",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// The paths the page links to on its own origin, without query or
/// fragment, each once, assets left out, at most [`PATHS`].
pub fn same_origin_paths(html: &str, base: &str) -> Vec<String> {
    let Ok(base) = url::Url::parse(base) else {
        return Vec::new();
    };
    let document = Html::parse_document(html);
    let Ok(selector) = Selector::parse("a[href], area[href], form[action]") else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for element in document.select(&selector).take(5000) {
        let href = element
            .value()
            .attr("href")
            .or_else(|| element.value().attr("action"))
            .unwrap_or_default();
        let Ok(target) = base.join(href) else {
            continue;
        };
        if target.origin() != base.origin() {
            continue;
        }
        let path = target.path().to_owned();
        if asset(&path) || out.contains(&path) {
            continue;
        }
        out.push(path);
        if out.len() == PATHS {
            break;
        }
    }
    out
}

fn prose_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"\b(GET|POST|PUT|PATCH|DELETE|HEAD)\s+(https?://[^\s/]+)?(/[A-Za-z0-9._~!$&'()*+,;=:@%/{}<>\-]*)")
            .expect("a valid pattern")
    })
}

/// A path as a route: variables written `:id`, `<id>` or `${id}` become
/// `{id}`, the punctuation prose ends it with goes, and `...` or an empty
/// path is no route.
fn route_path(raw: &str) -> Option<String> {
    if raw.contains("...") {
        return None;
    }
    let trimmed = raw.trim_end_matches(['.', ',', ';', ')', '\'', ':']);
    if trimmed.is_empty() {
        return None;
    }
    let path = trimmed.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<String> = path
        .split('/')
        .map(|segment| {
            let inner = segment
                .strip_prefix("${")
                .and_then(|s| s.strip_suffix('}'))
                .or_else(|| segment.strip_prefix(':'))
                .or_else(|| segment.strip_prefix('<').and_then(|s| s.strip_suffix('>')));
            match inner {
                Some(name) if !name.is_empty() => format!("{{{name}}}"),
                _ => segment.to_owned(),
            }
        })
        .collect();
    let path = segments.join("/");
    (path.starts_with('/') && !path.contains("//")).then_some(path)
}

fn push(out: &mut Vec<Route>, route: Route) {
    if out.len() < ROUTES && !out.contains(&route) {
        out.push(route);
    }
}

/// The routes the page's text names as a method and a path (`GET
/// /users/{id}`, `POST https://api.example.com/v1/items`), in order, each
/// once, at most [`ROUTES`]. Code blocks count as text.
pub fn prose_routes(html: &str) -> Vec<Route> {
    let document = Html::parse_document(html);
    let text = element_text(document.root_element());
    routes_in_text(&text[..floor(&text, TEXT_LIMIT)])
}

/// The same, over plain text.
pub fn routes_in_text(text: &str) -> Vec<Route> {
    let mut out = Vec::new();
    for found in prose_pattern().captures_iter(text) {
        let Some(path) = route_path(&found[3]) else {
            continue;
        };
        push(
            &mut out,
            Route {
                method: found[1].to_owned(),
                path,
                origin: found.get(2).map(|origin| origin.as_str().to_owned()),
            },
        );
    }
    out
}

fn floor(text: &str, limit: usize) -> usize {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// The scripts the page loads (`<script src>` and module preloads), as
/// addresses resolved against the page, best first for finding routes: an
/// app's own code before the frameworks it ships with. At most
/// [`SCRIPTS`].
pub fn script_sources(html: &str, base: &str) -> Vec<String> {
    let Ok(base) = url::Url::parse(base) else {
        return Vec::new();
    };
    let document = Html::parse_document(html);
    let Ok(selector) = Selector::parse("script[src], link[rel=modulepreload][href]") else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for element in document.select(&selector).take(500) {
        let src = element
            .value()
            .attr("src")
            .or_else(|| element.value().attr("href"))
            .unwrap_or_default();
        let Ok(url) = base.join(src) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let url = url.to_string();
        if !found.contains(&url) {
            found.push(url);
        }
    }
    let rank = |url: &str| {
        let lower = url.to_ascii_lowercase();
        if [
            "polyfill",
            "vendor",
            "framework",
            "runtime",
            "chunk-vendors",
        ]
        .iter()
        .any(|w| lower.contains(w))
        {
            2
        } else if [
            "/app", "/pages/", "/routes/", "search", "api", "graphql", "main",
        ]
        .iter()
        .any(|w| lower.contains(w))
        {
            0
        } else {
            1
        }
    };
    found.sort_by_key(|url| rank(url));
    found.truncate(SCRIPTS);
    found
}

fn literal_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"["'`](/(?:api|v\d+|graphql|rest|ajax|data|_next/data|json)[A-Za-z0-9._~!$&*+,;=:@%/{}\-]*)["'`?]"#)
            .expect("a valid pattern")
    })
}

fn method_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)method\s*:\s*["'`](GET|POST|PUT|PATCH|DELETE)["'`]"#)
            .expect("a valid pattern")
    })
}

/// The API routes a script's own text names as string literals (`"/api/
/// items"`, `` `/v1/users/${id}` ``), each with the method written within a
/// few lines of it (`method: "POST"`), `GET` when none is, `POST` for a
/// GraphQL path. At most [`ROUTES`]. A literal is only a lead: nothing here
/// says the route answers.
pub fn bundle_routes(script: &str) -> Vec<Route> {
    let script = &script[..floor(script, TEXT_LIMIT)];
    let mut out = Vec::new();
    for found in literal_pattern().captures_iter(script) {
        let Some(whole) = found.get(1) else {
            continue;
        };
        let raw = whole.as_str().replace("${", "{");
        let Some(path) = route_path(&raw) else {
            continue;
        };
        // The rest of the statement the literal is in, a few lines at most.
        let window = &script[whole.end()..floor(script, whole.end() + 160)];
        let after = window.split([';', '\n']).next().unwrap_or_default();
        let method = if path.to_ascii_lowercase().contains("graphql") {
            "POST".to_owned()
        } else {
            method_pattern()
                .captures(after)
                .map_or("GET".to_owned(), |m| m[1].to_ascii_uppercase())
        };
        push(
            &mut out,
            Route {
                method,
                path,
                origin: None,
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCS: &str = r#"<html><head><title>API</title><script src="/static/vendor.js"></script>
<script src="/static/app.js"></script><link rel="modulepreload" href="/static/routes/list.mjs"></head>
<body><h1>Fixture API</h1><h2>Items</h2><p>Use <code>GET /api/items</code> to list them, and
<code>GET /api/items/:id</code> or <code>GET /api/items/{id}/comments</code>. Create one with
POST https://api.fixture.test/v1/items. Paths like GET /api/... are examples.</p>
<a href="/docs/items?tab=1">Items</a><a href="/docs/items#top">again</a><a href="https://elsewhere.test/x">out</a>
<a href="/static/logo.png">logo</a><form action="/search"><input name="q"></form>
<h3>  </h3><table><tr><td>secret record value</td></tr></table></body></html>"#;

    #[test]
    fn headings_are_in_order_and_empty_ones_are_left_out() {
        let found = headings(DOCS);
        assert_eq!(
            found,
            [
                Heading {
                    level: 1,
                    text: "Fixture API".into()
                },
                Heading {
                    level: 2,
                    text: "Items".into()
                },
            ]
        );
    }

    #[test]
    fn prose_names_routes_with_their_variables() {
        let routes = prose_routes(DOCS);
        let pairs: Vec<(&str, &str, Option<&str>)> = routes
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str(), r.origin.as_deref()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("GET", "/api/items", None),
                ("GET", "/api/items/{id}", None),
                ("GET", "/api/items/{id}/comments", None),
                ("POST", "/v1/items", Some("https://api.fixture.test")),
            ]
        );
    }

    #[test]
    fn same_origin_paths_leave_out_queries_assets_and_other_sites() {
        assert_eq!(
            same_origin_paths(DOCS, "https://fixture.test/docs"),
            ["/docs/items", "/search"]
        );
    }

    #[test]
    fn scripts_come_app_first_and_vendors_last() {
        assert_eq!(
            script_sources(DOCS, "https://fixture.test/docs"),
            [
                "https://fixture.test/static/app.js",
                "https://fixture.test/static/routes/list.mjs",
                "https://fixture.test/static/vendor.js",
            ]
        );
    }

    #[test]
    fn bundle_literals_become_routes_with_the_method_written_next_to_them() {
        let script = r#"const a = fetch("/api/items"); const b = fetch(`/api/items/${id}`, {method: "DELETE"});
            axios.post('/api/items', body, {method: 'POST'}); const c = "/graphql"; const d = "/about";
            const e = "/api/..."; const f = '/v2/users?x=1';"#;
        let routes: Vec<(String, String)> = bundle_routes(script)
            .into_iter()
            .map(|r| (r.method, r.path))
            .collect();
        assert_eq!(
            routes,
            [
                ("GET".to_owned(), "/api/items".to_owned()),
                ("DELETE".to_owned(), "/api/items/{id}".to_owned()),
                ("POST".to_owned(), "/api/items".to_owned()),
                ("POST".to_owned(), "/graphql".to_owned()),
                ("GET".to_owned(), "/v2/users".to_owned()),
            ]
        );
    }
}
