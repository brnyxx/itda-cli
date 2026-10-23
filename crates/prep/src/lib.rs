//! Drafts site definitions from descriptions a site already publishes.
//!
//! A draft is a starting point for a person, never something installed
//! silently: every operation carries the evidence it came from, changes stay
//! changes, and whatever could not be translated is listed as a note instead of
//! guessed. The draft is checked with the same rules `itda` applies at install
//! time, so problems show up before anyone runs it.
mod confirm;
pub mod graphql;
pub mod guide;
mod har;
pub mod label;
mod openapi;
mod page;

pub use har::{from_har, query_session_key, redacted_recording};
pub use openapi::{from_openapi, is_openapi};
pub use page::{
    DESCRIPTION_PATH, Description, Page, embeds_lists, from_pages, from_pages_and_description,
};

use itda_types::error::{AppResult, usage};

use serde_norway::{Mapping, Value as Yaml};

/// Choices a person makes about a draft.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The site id; taken from the document's title when left out.
    pub name: Option<String>,
    /// Overrides the server URL in the document.
    pub base_url: Option<String>,
    /// Keep only operations whose path, name or tag contains this text.
    pub include: Option<String>,
    /// `ko` or `en` for the descriptions people read; the page's own
    /// language, or English, when left empty.
    pub lang: String,
    /// What the GraphQL endpoints the pages or calls named answered the
    /// introspection query with, fetched by the caller. Each becomes reads.
    pub schemas: Vec<graphql::Schema>,
    /// The pages were fetched signed in through the site's own form: every
    /// action needs that session, and the definition says how to sign in.
    pub session: Option<SessionAuth>,
    /// A session the site keeps in the page's own storage and sends as a
    /// header, recognised from the sign-in window: the calls that sent it
    /// get a `kind: header` session whose value lives in the keyring under
    /// `storage:<key>`, not a static `store:` token.
    pub storage: Option<StorageAuth>,
    /// Paths of the page's own frames that answered with a redirect to a
    /// sign-in and so were not read; the note for each says it leads to a
    /// sign-in instead of only that it was not read.
    pub frames_to_sign_in: Vec<String>,
}

/// A sign-in the pages were read with: the cookie that shows it (empty when
/// none could be told), or the query key the site carries the session in
/// (which then takes the cookie's place), and the form that gave it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionAuth {
    pub cookie: String,
    /// The query key every signed-in request carries, such as `sid`, when
    /// the site keeps its session in the address rather than a cookie.
    pub query: Option<String>,
    pub login: itda_spec::LoginForm,
}

/// A session the site keeps in browser storage and sends as a header: the
/// storage key it is under, the header the scripts put it in, and the prefix
/// in front of it there (`Bearer `). The value itself never reaches a draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StorageAuth {
    pub key: String,
    pub header: String,
    pub prefix: String,
}

/// The schemas' reads, after what the source itself gave. They share one
/// path, so they go in directly rather than through the `(method, path)`
/// checks that keep two sources from drafting one call twice.
pub(crate) fn add_schemas(parts: &mut Parts, options: &Options, lang: &str) {
    if options.schemas.is_empty() {
        return;
    }
    let Ok(base) = url::Url::parse(&parts.base_url) else {
        return;
    };
    let origin = base.origin();
    let prefix = base.path().trim_end_matches('/').to_owned();
    let mut names: std::collections::BTreeSet<String> = parts
        .operations
        .iter()
        .filter_map(|op| op.get("name").and_then(Yaml::as_str).map(str::to_owned))
        .collect();
    for schema in &options.schemas {
        let rest = url::Url::parse(&schema.url)
            .ok()
            .filter(|url| url.origin() == origin)
            .and_then(|url| url.path().strip_prefix(prefix.as_str()).map(str::to_owned))
            .filter(|rest| rest.is_empty() || rest.starts_with('/'));
        let Some(rest) = rest else {
            parts.notes.push(format!(
                "{}: left out its schema; it is not under {}.",
                schema.url, parts.base_url
            ));
            continue;
        };
        let path = if rest.is_empty() { "/" } else { rest.as_str() };
        let reads = graphql::operations(
            schema,
            path,
            lang,
            options.include.as_deref(),
            &mut names,
            &mut parts.notes,
        );
        parts.operations.extend(reads);
    }
}

/// A drafted definition and what the person should review.
#[derive(Debug)]
pub struct Draft {
    pub site: String,
    pub yaml: String,
    pub operations: usize,
    pub reads: usize,
    /// Of the reads, how many check nothing but a page's title: a page that
    /// showed no records, data, form or feed. A draft with no other read
    /// gives an agent little.
    pub thin_reads: usize,
    /// What was skipped or simplified, one line each.
    pub notes: Vec<String>,
    /// What the install-time checks found, one line each.
    pub problems: Vec<String>,
    /// What the site's API description added, when the pages had one.
    pub described: Option<Described>,
}

/// What an API description contributed to a page draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Described {
    pub url: String,
    /// Reads merged into the draft, after dropping those the pages already had.
    pub reads: usize,
    /// Changes the description lists; noted, never drafted from a page.
    pub changes: usize,
}

/// A lower-case id made of letters, digits and single dashes, starting with a
/// letter, as definitions require.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in text.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            out.push('-');
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.ends_with('-') {
                out.push('-');
            }
            previous_lower = false;
        }
    }
    let trimmed = out
        .trim_matches('-')
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
    let mut id: String = trimmed.chars().take(64).collect();
    while id.ends_with('-') {
        id.pop();
    }
    id
}

/// A draft before its final checks: what one source found.
pub(crate) struct Parts {
    pub site: String,
    /// The site's human name, when the source gives one.
    pub title: String,
    pub base_url: String,
    pub auth: Vec<Yaml>,
    pub operations: Vec<Mapping>,
    /// The `models` block: the fields each typed read keeps, by name.
    pub models: Mapping,
    pub notes: Vec<String>,
    pub source: String,
}

impl Parts {
    pub(crate) fn finish(self) -> Draft {
        finish(self, "")
    }

    /// What makes two drafted calls the same call: the method and the path,
    /// under the operation's own base address when it has one.
    fn key(entry: &Mapping) -> (String, String, String) {
        let field = |name: &str| {
            entry
                .get(name)
                .and_then(Yaml::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        (field("method"), field("base_url"), field("path"))
    }

    /// Add what another source found about the same site: operations the
    /// first source did not have, auth entries by name, and its notes.
    /// Returns how many operations were new.
    fn merge(&mut self, other: Parts) -> usize {
        let seen: std::collections::BTreeSet<_> = self.operations.iter().map(Self::key).collect();
        let before = self.operations.len();
        let mut names: std::collections::BTreeSet<String> = self
            .operations
            .iter()
            .filter_map(|op| op.get("name").and_then(Yaml::as_str).map(str::to_owned))
            .collect();
        for mut op in other.operations {
            if seen.contains(&Self::key(&op)) {
                continue;
            }
            // `GET /api/crates/serde` and the page `/crates/serde` both want
            // `get-serde`; the second becomes `get-serde-2`.
            let name = op
                .get("name")
                .and_then(Yaml::as_str)
                .unwrap_or("action")
                .to_owned();
            let unique = har::unique(&mut names, &name);
            if unique != name {
                op.insert("name".into(), unique.into());
            }
            self.operations.push(op);
        }
        for strategy in other.auth {
            let name = strategy.get("name").and_then(Yaml::as_str);
            let known = self
                .auth
                .iter()
                .any(|a| a.get("name").and_then(Yaml::as_str) == name);
            if !known {
                self.auth.push(strategy);
            }
        }
        for (name, model) in other.models {
            self.models.entry(name).or_insert(model);
        }
        self.notes.extend(other.notes);
        self.operations.len() - before
    }
}

/// The addresses of the pages a recording holds under `base_url`, the ones
/// [`from_recording`] drafts from, so they can be fetched again later. A
/// session the site carries in the query is taken off each address.
pub fn pages_in_recording(text: &[u8], base_url: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_slice(text).unwrap_or_default();
    let session_key = doc["log"]["entries"]
        .as_array()
        .and_then(|entries| query_session_key(entries));
    let mut pages: Vec<String> = Vec::new();
    for entry in doc["log"]["entries"].as_array().into_iter().flatten() {
        if !har::html_page(entry) {
            continue;
        }
        let Some(url) = entry["request"]["url"].as_str() else {
            continue;
        };
        let url = url.split('#').next().unwrap_or(url);
        let url = match &session_key {
            Some(key) => itda_types::url::without_query_key(url, key),
            None => url.to_owned(),
        };
        if pages.contains(&url) || !url.starts_with(base_url) {
            continue;
        }
        pages.push(url);
        if pages.len() == 20 {
            break;
        }
    }
    pages
}

/// A recording of a person using a site: the calls its pages made, drafted
/// as `from_har` does, plus the pages themselves, drafted as `from_pages`
/// does, so a site that renders everything on the server still gets its
/// lists, forms and feeds. `source` names the command for the header.
pub fn from_recording(text: &[u8], options: &Options, source: &str) -> AppResult<Draft> {
    let mut parts = har::har_parts(text, options)?;
    let doc: serde_json::Value = serde_json::from_slice(text).unwrap_or_default();
    // A session carried in the query is auth, never an input with the
    // session as its default: it comes off every page address first.
    let session_key = doc["log"]["entries"]
        .as_array()
        .and_then(|entries| query_session_key(entries));
    let mut pages: Vec<Page> = Vec::new();
    for entry in doc["log"]["entries"].as_array().into_iter().flatten() {
        if !har::html_page(entry) {
            continue;
        }
        let (Some(url), Some(html)) = (
            entry["request"]["url"].as_str(),
            entry["response"]["content"]["text"].as_str(),
        ) else {
            continue;
        };
        let url = url.split('#').next().unwrap_or(url);
        let url = match &session_key {
            Some(key) => itda_types::url::without_query_key(url, key),
            None => url.to_owned(),
        };
        if pages.iter().any(|page| page.url == url) || !url.starts_with(&parts.base_url) {
            continue;
        }
        pages.push(Page {
            url,
            html: html.to_owned(),
            link: String::new(),
        });
        if pages.len() == 20 {
            parts
                .notes
                .push("itda analysed only the first 20 pages of the recording.".into());
            break;
        }
    }
    if !pages.is_empty() {
        // A session the site carries in the query makes the pages read
        // signed in too, so they carry `auth: session` like the calls; the
        // auth entry itself is already on `parts`, so the merge keeps that
        // one and only the per-operation auth comes from here.
        let session = url::Url::parse(&parts.base_url).ok().and_then(|base| {
            let prefix = base.path().trim_end_matches('/').to_owned();
            har::query_session_auth(
                &doc["log"]["entries"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
                &base.origin(),
                &prefix,
            )
        });
        let page_options = Options {
            name: Some(parts.site.clone()),
            base_url: Some(parts.base_url.clone()),
            include: options.include.clone(),
            lang: options.lang.clone(),
            schemas: Vec::new(),
            session,
            storage: None,
            frames_to_sign_in: options.frames_to_sign_in.clone(),
        };
        parts.merge(page::page_parts(&pages, &page_options)?);
    }
    add_schemas(&mut parts, options, &label::language(&options.lang, ""));
    if parts.operations.is_empty() {
        return Err(har::no_operations());
    }
    let host = url::Url::parse(&parts.base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    parts.source = format!("{source} from a recording of {host}");
    Ok(parts.finish())
}

/// A ready-made definition for software a site runs (`sites/software/
/// <name>.yaml`), filled in for one site.
#[derive(Clone, Debug)]
pub struct Fill<'a> {
    /// The template's text.
    pub template: &'a str,
    /// The software's name as people say it (`GitLab`), for the header.
    pub software: &'a str,
    /// The site's API base, in place of the template's placeholder: the
    /// instance's origin plus its context path and the API's own path.
    pub base_url: &'a str,
}

/// A template, filled in for one site, with what its pages show merged in:
/// the template's site name, auth, models and operations come first; the
/// pages add the reads and changes the template does not have (the same
/// method and path under the same base is drafted once), under their own
/// base address when it differs from the API's. The site is named by
/// `options.name`, else after the host. `pages` may be empty.
pub fn from_template(
    fill: &Fill<'_>,
    pages: &[Page],
    description: Option<&Description>,
    options: &Options,
) -> AppResult<Draft> {
    let doc: Mapping = serde_norway::from_str(fill.template).map_err(|error| {
        usage(
            format!("The {} template does not parse: {error}", fill.software),
            "Report this with `itda feedback`.",
        )
    })?;
    let site_block = doc.get("site").and_then(Yaml::as_mapping);
    let field = |name: &str| {
        site_block
            .and_then(|site| site.get(name))
            .and_then(Yaml::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let template_name = field("name");
    let base = url::Url::parse(fill.base_url.trim_end_matches('/')).map_err(|_| {
        usage(
            format!("{} is not a URL", fill.base_url),
            "Pass an address like https://example.com/api.",
        )
    })?;
    let base_url = base.as_str().trim_end_matches('/').to_owned();
    let site = options
        .name
        .as_deref()
        .map(slug)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| har::site_name(base.host_str().unwrap_or_default()));
    let auth: Vec<Yaml> = doc
        .get("auth")
        .and_then(Yaml::as_sequence)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut entry| {
            // `store:<template>/<key>` names the site the secret belongs to.
            if let Yaml::Mapping(map) = &mut entry
                && let Some(Yaml::String(from)) = map.get_mut("value_from")
                && let Some(rest) = from.strip_prefix(&format!("store:{template_name}/"))
            {
                *from = format!("store:{site}/{rest}");
            }
            entry
        })
        .collect();
    let operations: Vec<Mapping> = doc
        .get("operations")
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|op| op.as_mapping().cloned())
        .collect();
    let models = doc
        .get("models")
        .and_then(Yaml::as_mapping)
        .cloned()
        .unwrap_or_default();
    let host = base.host_str().unwrap_or_default().to_owned();
    let mut parts = Parts {
        site: site.clone(),
        title: field("title"),
        base_url,
        auth,
        operations,
        models,
        notes: Vec::new(),
        source: format!(
            "`itda setup` from the {} template for {host}",
            fill.software
        ),
    };
    let mut described = None;
    if !pages.is_empty() {
        let page_options = Options {
            name: Some(site),
            base_url: None,
            include: options.include.clone(),
            lang: options.lang.clone(),
            schemas: options.schemas.clone(),
            session: options.session.clone(),
            storage: None,
            frames_to_sign_in: options.frames_to_sign_in.clone(),
        };
        let (mut found, from_description) =
            page::page_parts_with_description(pages, description, &page_options)?;
        described = from_description;
        if found.base_url != parts.base_url {
            for op in &mut found.operations {
                op.insert("base_url".into(), found.base_url.clone().into());
            }
        }
        parts.merge(found);
        parts.source = format!(
            "`itda setup` from the {} template and a page of {host}",
            fill.software
        );
    }
    let mut draft = parts.finish();
    draft.described = described;
    Ok(draft)
}

/// A read that checks only the page's title: `required_html_fields` on it
/// and no records, embedded data or format to check.
fn thin_read(op: &Mapping) -> bool {
    if op.get("effect").and_then(Yaml::as_str) != Some("read") {
        return false;
    }
    let Some(Yaml::Mapping(response)) = op.get("response") else {
        return false;
    };
    let only_title = response
        .get("html")
        .and_then(|html| html.get("page"))
        .and_then(Yaml::as_mapping)
        .is_some_and(|page| page.len() == 1 && page.contains_key("title"));
    only_title
        && ["items", "ssr_json", "required_pointers"]
            .iter()
            .all(|key| !response.contains_key(*key))
}

/// True for the error [`from_recording`] and [`from_har`] give when the
/// recording held nothing to draft from, as opposed to a recording that
/// could not be read at all.
pub fn drafted_nothing(error: &itda_types::error::AppError) -> bool {
    error.msg == har::no_operations().msg
}

/// Writes the definition and checks it with the rules `itda` applies at
/// install time. The parts' `source` says what drafted it, for the header
/// comment; `title` is the site's human name when the source gave one.
pub(crate) fn finish(parts: Parts, title: &str) -> Draft {
    let Parts {
        site,
        title: own_title,
        base_url,
        auth,
        mut operations,
        models,
        notes,
        source,
    } = parts;
    confirm::fill(&mut operations, &models);
    let title = if title.is_empty() {
        own_title.as_str()
    } else {
        title
    };
    let (site, base_url, source) = (site.as_str(), base_url.as_str(), source.as_str());
    let mut site_map = Mapping::new();
    site_map.insert("name".into(), site.into());
    if !title.is_empty() {
        site_map.insert("title".into(), title.into());
    }
    site_map.insert("base_url".into(), base_url.into());
    let mut root = Mapping::new();
    root.insert("spec_version".into(), 1.into());
    root.insert("site".into(), Yaml::Mapping(site_map));
    if !auth.is_empty() {
        root.insert("auth".into(), Yaml::Sequence(auth));
    }
    let reads = operations
        .iter()
        .filter(|op| op.get("effect").and_then(Yaml::as_str) == Some("read"))
        .count();
    let thin_reads = operations.iter().filter(|op| thin_read(op)).count();
    let count = operations.len();
    root.insert(
        "operations".into(),
        Yaml::Sequence(operations.into_iter().map(Yaml::Mapping).collect()),
    );
    if !models.is_empty() {
        root.insert("models".into(), Yaml::Mapping(models));
    }
    let body = serde_norway::to_string(&Yaml::Mapping(root)).unwrap_or_default();
    let yaml = format!(
        "# Drafted by {source}. Review every operation before use:\n# check names, effects and response checks, and remove what agents should not call.\n{body}"
    );
    let problems = match itda_spec::parse(yaml.as_bytes()) {
        Ok(spec) => itda_policy::spec_problems(&spec)
            .into_iter()
            .map(|p| format!("{}: {}", p.field, p.detail))
            .collect(),
        Err(error) => vec![error.msg],
    };
    Draft {
        site: site.to_owned(),
        yaml,
        operations: count,
        reads,
        thin_reads,
        notes,
        problems,
        described: None,
    }
}

pub(crate) fn insert_after(map: &mut Mapping, after: &str, key: &str, value: Yaml) {
    let mut out = Mapping::new();
    for (k, v) in std::mem::take(map) {
        let matched = k.as_str() == Some(after);
        out.insert(k, v);
        if matched {
            out.insert(key.into(), value.clone());
        }
    }
    *map = out;
}

/// `name` first and `evidence` after `effect`, as people write them.
/// A `pagination` block: the style, where the next marker is (`next`, a
/// pointer or `header:link`, empty for none), the pointer that must be true
/// for another page (`more`, empty for none) and the input it goes back in.
pub(crate) fn pagination_yaml(
    style: &str,
    next: &str,
    more: &str,
    input: &str,
    in_body: bool,
) -> Yaml {
    let mut block = Mapping::new();
    block.insert("style".into(), style.into());
    if !next.is_empty() {
        block.insert("next".into(), next.into());
    }
    if !more.is_empty() {
        block.insert("more".into(), more.into());
    }
    let mut send = Mapping::new();
    send.insert(if in_body { "body" } else { "query" }.into(), input.into());
    block.insert("send".into(), Yaml::Mapping(send));
    Yaml::Mapping(block)
}

/// The response field a next cursor is read from, when `keys` names one
/// (`next_cursor`, `nextPageToken`, `end_cursor`...), as a JSON pointer.
pub(crate) fn next_cursor_key<'a>(keys: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    keys.filter(|key| {
        let plain = key.to_ascii_lowercase().replace(['-', '_'], "");
        matches!(
            plain.as_str(),
            "nextcursor"
                | "nextpagetoken"
                | "nexttoken"
                | "endcursor"
                | "continuation"
                | "continuationtoken"
                | "nextcontinuationtoken"
                | "next"
                | "cursor"
        )
    })
    .max_by_key(|key| {
        // `next_cursor` says more than `next`; `cursor` least of all.
        let plain = key.to_ascii_lowercase().replace(['-', '_'], "");
        match plain.as_str() {
            "cursor" => 0,
            "next" => 1,
            _ => 2,
        }
    })
}

/// The query input a cursor goes back in, when `names` has one that says
/// so (`cursor`, `page_token`, `after`...) and no page number.
pub(crate) fn cursor_input<'a>(names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut found = None;
    for name in names {
        let plain = name.to_ascii_lowercase().replace(['-', '_'], "");
        if matches!(
            plain.as_str(),
            "cursor"
                | "after"
                | "pagetoken"
                | "nextpagetoken"
                | "nexttoken"
                | "continuation"
                | "continuationtoken"
                | "startingafter"
        ) {
            if found.is_some() {
                return None;
            }
            found = Some(name);
        }
    }
    found
}

/// The page-number or offset input among `names`, with its style.
pub(crate) fn counted_input<'a>(
    names: impl Iterator<Item = &'a str>,
) -> Option<(&'a str, &'static str)> {
    let mut found = None;
    for name in names {
        let style = match name.to_ascii_lowercase().as_str() {
            "page" | "page_no" | "pageno" | "page_num" | "pagenum" | "page_index" | "pageindex" => {
                "page"
            }
            "offset" | "skip" | "from" | "start" => "offset",
            _ => continue,
        };
        if found.is_some() {
            return None;
        }
        found = Some((name, style));
    }
    found
}

pub(crate) fn reorder(entry: Mapping) -> Mapping {
    let order = [
        "name",
        "desc",
        "method",
        "path",
        "auth",
        "effect",
        "evidence",
        "headers",
        "preflight",
        "params",
        "query",
        "body",
        "pagination",
        "response",
        "model",
    ];
    let mut out = Mapping::new();
    for key in order {
        if let Some(value) = entry.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slugs_are_valid_ids() {
        assert_eq!(slug("listPetsByOwner"), "list-pets-by-owner");
        assert_eq!(slug("GET /v1/pets/{id}"), "get-v1-pets-id");
        assert_eq!(slug("2024 Store API"), "store-api");
        assert_eq!(slug("HTTPServer"), "httpserver");
        assert!(itda_types::util::valid_id(&slug("Pet_Store  API!!")));
    }
}
