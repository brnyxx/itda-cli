//! Fetched pages to a draft definition.
//!
//! A page shows what it is made of: a table or a list of records, data the
//! server embedded for its scripts, forms, feeds and page links. Each becomes
//! an action with a response check, so `itda check` can tell when the page
//! changes. When the site also publishes an API description, its reads join
//! the draft. Pages and descriptions are fetched by the caller with the honest
//! client; nothing here reaches the network.
use crate::har::{Kind, action_name, site_name, template, unique};
use crate::label::{Label, describe, language};
use crate::openapi::openapi_parts;
use crate::{Described, Draft, Options, Parts, reorder, slug};
use itda_html::evidence;
use itda_types::error::{AppResult, usage};
use itda_types::util::secret_field;
use serde_norway::{Mapping, Value as Yaml};
use std::collections::BTreeSet;

/// One fetched page.
#[derive(Clone, Debug)]
pub struct Page {
    pub url: String,
    pub html: String,
}

/// An API description the site publishes, as fetched: the one a page linked
/// to, or the one [`DESCRIPTION_PATH`] answered with.
#[derive(Clone, Debug)]
pub struct Description {
    pub url: String,
    pub bytes: Vec<u8>,
}

/// Changes an API description lists are noted one per line up to here; a
/// large API gets a count instead of a wall of notes.
const LISTED_CHANGES: usize = 10;

/// The one same-origin path a caller asks for when no page links to a
/// description. Every request to a site waits its turn at the gate, so more
/// paths would cost every site that publishes nothing a second or more each.
pub const DESCRIPTION_PATH: &str = "/openapi.json";

/// Embedded payloads in the order a definition should prefer them.
const SSR_KEYS: &[&str] = &[
    "__NEXT_DATA__",
    "__remixContext",
    "__NEXT_F__",
    "__APOLLO_STATE__",
    "__NUXT_DATA__",
    "__INITIAL_STATE__",
    "__PRELOADED_STATE__",
    "__STATE__",
    "INIT_DATA",
];

pub fn from_pages(pages: &[Page], options: &Options) -> AppResult<Draft> {
    from_pages_and_description(pages, None, options)
}

/// The pages, plus the reads of the API description the site publishes.
/// Changes the description lists are noted, never drafted from a page, and a
/// description that cannot be read leaves the page draft as it is.
pub fn from_pages_and_description(
    pages: &[Page],
    description: Option<&Description>,
    options: &Options,
) -> AppResult<Draft> {
    let mut parts = page_parts(pages, options)?;
    let described = description.and_then(|d| merge_description(&mut parts, d, options));
    let lang = language(&options.lang, &page_lang(&pages[0].html));
    crate::add_schemas(&mut parts, options, &lang);
    if parts.operations.is_empty() {
        return Err(usage(
            "No actions could be drafted",
            "Check that the pages are under the site's address, or loosen --include.",
        ));
    }
    let mut draft = parts.finish();
    draft.described = described;
    Ok(draft)
}

/// Reads from the description join the page's operations, under the page's
/// site and base address; each change becomes one note. `None` when the
/// description could not be used, with a note saying why.
fn merge_description(
    parts: &mut Parts,
    description: &Description,
    options: &Options,
) -> Option<Described> {
    let url = &description.url;
    let api_options = Options {
        name: Some(parts.site.clone()),
        base_url: None,
        include: options.include.clone(),
        lang: options.lang.clone(),
        schemas: Vec::new(),
    };
    let mut api = match openapi_parts(&description.bytes, &api_options, Some(url)) {
        Ok(api) => api,
        Err(error) => {
            parts.notes.push(format!(
                "{url} could not be read as an API description ({}); drafted from the pages alone.",
                error.msg
            ));
            return None;
        }
    };
    // Operation paths are relative to the description's server; they join the
    // draft only when that server is under the page's address.
    let prefix = match api.base_url.strip_prefix(parts.base_url.as_str()) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest.to_owned(),
        _ => {
            parts.notes.push(format!(
                "{url} describes an API at {}, not under {}; `itda draft openapi {url}` makes a definition of its own.",
                api.base_url, parts.base_url
            ));
            return None;
        }
    };
    let (reads, changes): (Vec<Mapping>, Vec<Mapping>) = std::mem::take(&mut api.operations)
        .into_iter()
        .partition(|op| op.get("effect").and_then(Yaml::as_str) == Some("read"));
    let label = |op: &Mapping| {
        format!(
            "{} {}",
            op.get("method").and_then(Yaml::as_str).unwrap_or_default(),
            op.get("path").and_then(Yaml::as_str).unwrap_or_default()
        )
    };
    // A read whose input is a credential would fail the install-time checks
    // and take the page's own actions down with it.
    let (reads, keyed): (Vec<Mapping>, Vec<Mapping>) = reads
        .into_iter()
        .partition(|op| credential_inputs(op).is_empty());
    let change_labels: Vec<String> = changes.iter().map(label).collect();
    let left_out: Vec<String> = keyed.iter().map(label).collect();
    // Notes about operations that are not drafted would only confuse: the
    // changes, whether drafted or skipped, and the reads left out here.
    api.notes.retain(|note| {
        !["POST ", "PUT ", "PATCH ", "DELETE "]
            .iter()
            .any(|method| note.starts_with(method))
            && !left_out.iter().any(|l| note.starts_with(l.as_str()))
    });
    for op in &keyed {
        let inputs = credential_inputs(op);
        parts.notes.push(match inputs.as_slice() {
            [one] => format!(
                "{}: left out; its input {one} is a credential, which goes in auth, never through the agent.",
                label(op)
            ),
            many => format!(
                "{}: left out; its inputs {} are credentials, which go in auth, never through the agent.",
                label(op),
                many.join(", ")
            ),
        });
    }
    for label in change_labels.iter().take(LISTED_CHANGES) {
        parts.notes.push(format!(
            "{label}: a change the API description lists; not drafted from a page."
        ));
    }
    if change_labels.len() > LISTED_CHANGES {
        parts.notes.push(format!(
            "...and {} more changes, left out the same way.",
            change_labels.len() - LISTED_CHANGES
        ));
    }
    if !change_labels.is_empty() {
        parts.notes.push(format!(
            "`itda draft openapi {url}` drafts the changes for review."
        ));
    }
    api.operations = reads
        .into_iter()
        .map(|mut op| {
            if !prefix.is_empty()
                && let Some(path) = op.get("path").and_then(Yaml::as_str)
            {
                let joined = format!("{prefix}{path}");
                op.insert("path".into(), joined.into());
            }
            op
        })
        .collect();
    let reads = parts.merge(api);
    parts
        .notes
        .retain(|note| !note.contains(&format!("links to an API description at {url};")));
    parts.source = format!("{} and the API description at {url}", parts.source);
    Some(Described {
        url: url.clone(),
        reads,
        changes: change_labels.len(),
    })
}

/// The pages as operations, without the final checks.
pub(crate) fn page_parts(pages: &[Page], options: &Options) -> AppResult<Parts> {
    let first = pages.first().ok_or_else(|| {
        usage(
            "No page was given",
            "Pass the address of a page you use, for example a board or a search page.",
        )
    })?;
    let base = match &options.base_url {
        Some(base) => url::Url::parse(base.trim_end_matches('/')).map_err(|_| {
            usage(
                "--base-url is not a URL",
                "Pass an address like https://example.com.",
            )
        })?,
        None => {
            let mut url = page_url(&first.url)?;
            url.set_path("/");
            url.set_query(None);
            url
        }
    };
    let origin = base.origin();
    let prefix = base.path().trim_end_matches('/').to_owned();
    let base_url = format!("{}{prefix}", origin.ascii_serialization());
    let site = options
        .name
        .as_deref()
        .map(slug)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| site_name(base.host_str().unwrap_or_default()));

    let mut notes = Vec::new();
    let mut operations: Vec<Mapping> = Vec::new();
    let mut names = BTreeSet::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut sign_in = false;
    for page in pages {
        let url = page_url(&page.url)?;
        let Some(rest) = url
            .path()
            .strip_prefix(prefix.as_str())
            .filter(|_| url.origin() == origin)
        else {
            notes.push(format!(
                "{}: left out; it is not under {base_url}.",
                page.url
            ));
            continue;
        };
        if let Some(filter) = &options.include
            && !rest.to_lowercase().contains(&filter.to_lowercase())
        {
            continue;
        }
        let (path, params) = template(rest);
        let mut push = |entry: Mapping, notes: &mut Vec<String>| {
            let key = (
                entry
                    .get("method")
                    .and_then(Yaml::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                entry
                    .get("path")
                    .and_then(Yaml::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            );
            if !seen.insert(key.clone()) {
                notes.push(format!(
                    "{} {}: seen on more than one page; drafted once.",
                    key.0, key.1
                ));
                return;
            }
            operations.push(reorder(entry));
        };

        let lang = language(&options.lang, &page_lang(&page.html));
        let page_title = title_of(&page.html);
        push(
            page_action(page, &url, &path, &params, &mut names, &mut notes, &lang),
            &mut notes,
        );
        for form in evidence::forms(&page.html, url.as_str()) {
            let label = format!("{} form on {}", form.method, url.path());
            if form.password {
                sign_in = true;
                notes.push(format!(
                    "{label}: a sign-in form; left out. Sign in with `itda login {site} --browser` once the definition is installed."
                ));
                continue;
            }
            if form.multipart {
                notes.push(format!(
                    "{label}: skipped; it uploads files, which drafts do not cover yet."
                ));
                continue;
            }
            let Ok(action) = url::Url::parse(&form.action) else {
                continue;
            };
            let Some(rest) = action
                .path()
                .strip_prefix(prefix.as_str())
                .filter(|_| action.origin() == origin)
            else {
                notes.push(format!(
                    "{label}: left out; it sends to {}, not {base_url}.",
                    action.origin().ascii_serialization()
                ));
                continue;
            };
            if form.controls.iter().all(|c| c.hidden) && form.method == "GET" {
                continue;
            }
            let (path, params) = template(rest);
            // A form that changes the site almost always wants an account.
            sign_in |= form.method == "POST";
            push(
                form_action(
                    &form,
                    &url,
                    &path,
                    &params,
                    &mut names,
                    &mut notes,
                    &page_title,
                    &lang,
                ),
                &mut notes,
            );
        }
        for feed in evidence::feeds(&page.html, url.as_str()) {
            let Ok(feed_url) = url::Url::parse(&feed.url) else {
                continue;
            };
            let Some(rest) = feed_url
                .path()
                .strip_prefix(prefix.as_str())
                .filter(|_| feed_url.origin() == origin)
            else {
                continue;
            };
            let mut entry = Mapping::new();
            entry.insert("name".into(), unique(&mut names, "feed").into());
            entry.insert(
                "desc".into(),
                describe(&Label::Feed { title: &feed.title }, &lang).into(),
            );
            entry.insert("method".into(), "GET".into());
            entry.insert("path".into(), rest.into());
            entry.insert("effect".into(), "read".into());
            entry.insert(
                "evidence".into(),
                format!("{} links to it as an alternate feed", url.path()).into(),
            );
            let mut response = Mapping::new();
            response.insert("format".into(), "xml".into());
            entry.insert("response".into(), Yaml::Mapping(response));
            push(entry, &mut notes);
        }
        if let Some(link) = evidence::openapi_link(&page.html, url.as_str()) {
            notes.push(format!(
                "{} links to an API description at {link}; `itda draft openapi` makes a fuller definition from it.",
                url.path()
            ));
        }
    }
    let mut auth = Vec::new();
    if sign_in {
        let mut strategy = Mapping::new();
        strategy.insert("name".into(), "session".into());
        strategy.insert("kind".into(), "browser".into());
        auth.push(Yaml::Mapping(strategy));
        notes.push(format!(
            "Changes use auth: session, your browser sign-in: run `itda login {site} --browser` once after installing. Add it to reads that need an account too."
        ));
    }
    let host = base.host_str().unwrap_or_default().to_owned();
    Ok(Parts {
        site,
        title: String::new(),
        base_url,
        auth,
        operations,
        notes,
        source: format!("`itda draft page` from pages on {host}"),
    })
}

/// Input names of the operation that look like credentials, as the
/// install-time checks judge them.
fn credential_inputs(op: &Mapping) -> Vec<String> {
    fn keys(value: Option<&Yaml>) -> Vec<String> {
        value
            .and_then(Yaml::as_mapping)
            .map(|m| {
                m.keys()
                    .filter_map(Yaml::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    let body = op.get("body").and_then(Yaml::as_mapping);
    keys(op.get("params"))
        .into_iter()
        .chain(keys(op.get("query")))
        .chain(keys(body.and_then(|b| b.get("json"))))
        .chain(keys(body.and_then(|b| b.get("form"))))
        .filter(|name| secret_field(name))
        .collect()
}

fn page_url(raw: &str) -> AppResult<url::Url> {
    url::Url::parse(raw)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .ok_or_else(|| {
            usage(
                format!("{raw} is not a page address"),
                "Pass an http or https address.",
            )
        })
}

/// The page itself as a read: its records, its embedded data, or at least
/// its title, so the action fails when the page changes.
/// The page's `<title>`, trimmed of a site suffix such as ` - Example`.
fn title_of(html: &str) -> String {
    let title = itda_html::Field {
        selector: "title".into(),
        attribute: String::new(),
    }
    .extract(&itda_html::parse_document(html))
    .unwrap_or_default();
    let short = [" - ", " | ", " · ", " — ", " – "]
        .iter()
        .filter_map(|sep| title.split(sep).next())
        .min_by_key(|s| s.len())
        .unwrap_or(&title)
        .trim();
    if short.len() >= 3 {
        short.to_owned()
    } else {
        title.trim().to_owned()
    }
}

/// The `lang` attribute of the page, when it has one.
fn page_lang(html: &str) -> String {
    itda_html::Field {
        selector: "html".into(),
        attribute: "lang".into(),
    }
    .extract(&itda_html::parse_document(html))
    .unwrap_or_default()
}

fn page_action(
    page: &Page,
    url: &url::Url,
    path: &str,
    params: &[String],
    names: &mut BTreeSet<String>,
    notes: &mut Vec<String>,
    lang: &str,
) -> Mapping {
    let title = evidence::title_field(&page.html);
    let page_title = title_of(&page.html);
    let name = if path == "/" {
        "home".to_owned()
    } else {
        action_name("GET", path)
    };
    let name = unique(names, &name);
    let mut entry = Mapping::new();
    entry.insert("name".into(), name.into());
    let subject = if page_title.is_empty() {
        url.path().to_owned()
    } else {
        page_title.clone()
    };
    entry.insert(
        "desc".into(),
        describe(&Label::Page { title: &subject }, lang).into(),
    );
    entry.insert("method".into(), "GET".into());
    entry.insert("path".into(), path.into());
    entry.insert("effect".into(), "read".into());
    if !params.is_empty() {
        entry.insert("params".into(), Yaml::Mapping(required_strings(params)));
    }
    let mut query = Mapping::new();
    for (key, value) in url.query_pairs() {
        if secret_field(&key) || query.contains_key(key.as_ref()) {
            continue;
        }
        let kind = Kind::of_text(&value);
        let mut field = Mapping::new();
        field.insert("type".into(), kind.name().into());
        field.insert(
            "default".into(),
            match kind {
                Kind::Int => value
                    .parse::<i64>()
                    .map(Yaml::from)
                    .unwrap_or(value.as_ref().into()),
                Kind::Bool => (value == "true").into(),
                _ => value.as_ref().into(),
            },
        );
        query.insert(key.as_ref().into(), Yaml::Mapping(field));
    }
    if let Some(key) = evidence::page_key(&page.html, url.as_str())
        && !query.contains_key(key.as_str())
    {
        let mut field = Mapping::new();
        field.insert("type".into(), "int".into());
        field.insert("default".into(), 1.into());
        query.insert(key.into(), Yaml::Mapping(field));
    }
    if !query.is_empty() {
        entry.insert("query".into(), Yaml::Mapping(query));
    }

    let mut response = Mapping::new();
    response.insert("format".into(), "html".into());
    let label = format!("GET {}", url.path());
    if let Some(extraction) = evidence::infer_extraction(&page.html, url.as_str()) {
        entry.insert(
            "desc".into(),
            describe(&Label::List { title: &subject }, lang).into(),
        );
        let count = extraction.fields.len();
        if let Some(title) = &title {
            response.insert(
                "required_html_fields".into(),
                fields_yaml(std::slice::from_ref(title)),
            );
        }
        response.insert("html".into(), extraction_yaml(&extraction));
        entry.insert(
            "evidence".into(),
            format!(
                "{label} shows one record per `{}` with {count} fields",
                extraction.items
            )
            .into(),
        );
    } else if let Some((key, pointer, pointers)) = embedded(&page.html) {
        entry.insert(
            "desc".into(),
            describe(&Label::Embedded { title: &subject }, lang).into(),
        );
        response.insert("ssr_json".into(), key.clone().into());
        if !pointer.is_empty() {
            response.insert("extract_pointer".into(), pointer.clone().into());
        }
        if !pointers.is_empty() {
            response.insert(
                "required_pointers".into(),
                Yaml::Sequence(pointers.into_iter().map(Yaml::from).collect()),
            );
        }
        entry.insert(
            "evidence".into(),
            format!("{label} embeds its data in the {key} script").into(),
        );
        notes.push(if pointer.is_empty() {
            format!(
                "{label}: returns the whole {key} payload; add extract_pointer and a model to keep only what agents need."
            )
        } else {
            format!(
                "{label}: returns the list at {pointer} in the {key} payload; add a model to keep only the fields agents need."
            )
        });
    } else if let Some(title) = &title {
        response.insert(
            "required_html_fields".into(),
            fields_yaml(std::slice::from_ref(title)),
        );
        let mut page_fields = Mapping::new();
        let mut field = Mapping::new();
        field.insert("selector".into(), title.selector.clone().into());
        field.insert("required".into(), true.into());
        page_fields.insert("title".into(), Yaml::Mapping(field));
        let mut html = Mapping::new();
        html.insert("page".into(), Yaml::Mapping(page_fields));
        response.insert("html".into(), Yaml::Mapping(html));
        entry.insert(
            "evidence".into(),
            format!(
                "{label} has a `{}`; no repeated records were found",
                title.selector
            )
            .into(),
        );
        notes.push(if evidence::script_shell(&page.html) {
            format!(
                "{label}: the page is drawn by scripts, so only its title is checked. Record yourself using it with `itda record` to draft the calls it makes."
            )
        } else {
            format!(
                "{label}: no table, list or embedded data was found; only the title is checked. Add html fields for what agents should read."
            )
        });
    } else {
        entry.insert("evidence".into(), format!("{label} was fetched").into());
        notes.push(format!(
            "{label}: no title, records or embedded data were found; add a response check by hand."
        ));
        return entry;
    }
    entry.insert("response".into(), Yaml::Mapping(response));
    entry
}

/// A form as an action: a search as a read, a post as a change whose tokens
/// come from the form page through a preflight step.
#[allow(clippy::too_many_arguments)]
fn form_action(
    form: &evidence::Form,
    page: &url::Url,
    path: &str,
    params: &[String],
    names: &mut BTreeSet<String>,
    notes: &mut Vec<String>,
    page_title: &str,
    lang: &str,
) -> Mapping {
    let read = form.method == "GET";
    let searches = form.controls.iter().any(|c| {
        !c.hidden && matches!(c.name.as_str(), "q" | "query" | "search" | "keyword" | "kw")
    });
    // `/board/search` and a search box on `/board` are both `search-board`.
    let name = match (read, searches) {
        (true, true) => {
            let subject = path
                .split('/')
                .chain(page.path().split('/'))
                .rfind(|s| !s.is_empty() && !s.starts_with('{') && !s.contains("search"))
                .map(|s| format!("search-{s}"))
                .unwrap_or_else(|| "search".into());
            slug(&subject)
        }
        _ => action_name(&form.method, path),
    };
    let name = unique(names, &slug(&name));
    let label = format!("{} {path}", form.method);
    let mut entry = Mapping::new();
    entry.insert("name".into(), name.into());
    let subject = if page_title.is_empty() {
        page.path().to_owned()
    } else {
        page_title.to_owned()
    };
    entry.insert(
        "desc".into(),
        describe(
            &if read {
                Label::Search { title: &subject }
            } else {
                Label::Submit { title: &subject }
            },
            lang,
        )
        .into(),
    );
    entry.insert("method".into(), form.method.clone().into());
    entry.insert("path".into(), path.into());
    if !read {
        entry.insert("auth".into(), "session".into());
    }
    entry.insert("effect".into(), if read { "read" } else { "write" }.into());
    entry.insert(
        "evidence".into(),
        format!("a form on {} sends {label}", page.path()).into(),
    );
    if !params.is_empty() {
        entry.insert("params".into(), Yaml::Mapping(required_strings(params)));
    }
    let mut inputs = Mapping::new();
    let mut tokens = Vec::new();
    let mut left_out = Vec::new();
    for control in &form.controls {
        if control.secret {
            if read {
                left_out.push(control.name.clone());
            } else {
                tokens.push(control.name.clone());
            }
            continue;
        }
        let mut field = Mapping::new();
        field.insert("type".into(), control.kind.into());
        if control.required {
            field.insert("required".into(), true.into());
        }
        if let Some(value) = &control.value {
            field.insert(
                "default".into(),
                match control.kind {
                    "int" => value
                        .parse::<i64>()
                        .map(Yaml::from)
                        .unwrap_or(value.as_str().into()),
                    "bool" => (value == "true" || value == "on").into(),
                    _ => value.as_str().into(),
                },
            );
        }
        inputs.insert(control.name.clone().into(), Yaml::Mapping(field));
    }
    if !tokens.is_empty() {
        let mut extract = Mapping::new();
        for token in &tokens {
            let key = Some(slug(token))
                .filter(|key| !key.is_empty())
                .unwrap_or_else(|| "token".into());
            extract.insert(
                key.clone().into(),
                format!("css:input[name=\"{token}\"]@value").into(),
            );
            let mut field = Mapping::new();
            field.insert("type".into(), "string".into());
            field.insert("default".into(), format!("{{{{step.form.{key}}}}}").into());
            inputs.insert(token.clone().into(), Yaml::Mapping(field));
        }
        let mut step = Mapping::new();
        step.insert("name".into(), "form".into());
        step.insert(
            "path".into(),
            match page.query() {
                Some(query) => format!("{}?{query}", page.path()),
                None => page.path().to_owned(),
            }
            .into(),
        );
        step.insert("extract".into(), Yaml::Mapping(extract));
        entry.insert(
            "preflight".into(),
            Yaml::Sequence(vec![Yaml::Mapping(step)]),
        );
        notes.push(format!(
            "{label}: {} come from {} through a preflight step, read fresh before each change.",
            tokens.join(", "),
            page.path()
        ));
    }
    if !left_out.is_empty() {
        notes.push(format!(
            "{label}: left out {}; tokens like these come from sign-in or a preflight step, never from the agent.",
            left_out.join(", ")
        ));
    }
    if read {
        if !inputs.is_empty() {
            entry.insert("query".into(), Yaml::Mapping(inputs));
        }
        notes.push(format!(
            "{label}: no response check yet; run it once, then declare what the result page shows."
        ));
    } else if !inputs.is_empty() {
        let mut body = Mapping::new();
        body.insert("form".into(), Yaml::Mapping(inputs));
        entry.insert("body".into(), Yaml::Mapping(body));
    }
    entry
}

/// The embedded payload to read, the pointer to the list inside it when
/// there is one, and up to three pointers that must exist.
fn embedded(html: &str) -> Option<(String, String, Vec<String>)> {
    let map = itda_html::extract_ssr_map(html);
    let key = SSR_KEYS
        .iter()
        .find(|key| map.contains_key(**key))
        .map(|key| (*key).to_owned())
        .or_else(|| map.keys().next().cloned())?;
    let root = map.get(&key)?;
    let escape = |k: &str| k.replace('~', "~0").replace('/', "~1");
    if let Some((pointer, records)) = list_in(root, "", 0) {
        // Keys every record has, so the check holds for any page of them.
        let shared: Vec<String> = records
            .iter()
            .filter_map(serde_json::Value::as_object)
            .map(|object| object.keys().cloned().collect::<BTreeSet<_>>())
            .reduce(|a, b| a.intersection(&b).cloned().collect())
            .unwrap_or_default()
            .into_iter()
            .filter(|k| !secret_field(k))
            .take(3)
            .map(|k| format!("/0/{}", escape(&k)))
            .collect();
        return Some((key, pointer, shared));
    }
    let pointers = match (key.as_str(), root) {
        ("__NEXT_DATA__", _) => vec!["/props".to_owned()],
        (_, serde_json::Value::Object(object)) => object
            .keys()
            .take(3)
            .map(|k| format!("/{}", escape(k)))
            .collect(),
        _ => Vec::new(),
    };
    Some((key, String::new(), pointers))
}

/// The largest list of at least three records (objects) in the payload, at
/// its JSON pointer; the shallowest wins a tie.
fn list_in<'a>(
    value: &'a serde_json::Value,
    pointer: &str,
    depth: usize,
) -> Option<(String, &'a Vec<serde_json::Value>)> {
    if depth > 12 {
        return None;
    }
    let escape = |k: &str| k.replace('~', "~0").replace('/', "~1");
    let mut best: Option<(String, &'a Vec<serde_json::Value>)> = None;
    let mut consider = |candidate: Option<(String, &'a Vec<serde_json::Value>)>| {
        if let Some((pointer, list)) = candidate
            && best.as_ref().is_none_or(|(_, top)| list.len() > top.len())
        {
            best = Some((pointer, list));
        }
    };
    match value {
        serde_json::Value::Array(items) => {
            if items.len() >= 3 && items.iter().all(serde_json::Value::is_object) {
                consider(Some((pointer.to_owned(), items)));
            }
            for (index, item) in items.iter().enumerate().take(50) {
                consider(list_in(item, &format!("{pointer}/{index}"), depth + 1));
            }
        }
        serde_json::Value::Object(object) => {
            for (name, child) in object.iter().take(200) {
                consider(list_in(
                    child,
                    &format!("{pointer}/{}", escape(name)),
                    depth + 1,
                ));
            }
        }
        _ => {}
    }
    best
}

/// Only what differs from the defaults, as people write extractions.
fn extraction_yaml(extraction: &itda_html::Extraction) -> Yaml {
    let mut out = Mapping::new();
    out.insert("items".into(), extraction.items.clone().into());
    let mut fields = Mapping::new();
    for (name, field) in &extraction.fields {
        let mut entry = Mapping::new();
        if !field.selector.is_empty() {
            entry.insert("selector".into(), field.selector.clone().into());
        }
        if !field.attribute.is_empty() {
            entry.insert("attribute".into(), field.attribute.clone().into());
        }
        if field.required {
            entry.insert("required".into(), true.into());
        }
        if field.absolute_url {
            entry.insert("absolute_url".into(), true.into());
        }
        fields.insert(name.clone().into(), Yaml::Mapping(entry));
    }
    out.insert("fields".into(), Yaml::Mapping(fields));
    out.insert("limit".into(), (extraction.limit as u64).into());
    Yaml::Mapping(out)
}

fn required_strings(params: &[String]) -> Mapping {
    let mut out = Mapping::new();
    for param in params {
        let mut field = Mapping::new();
        field.insert("type".into(), "string".into());
        field.insert("required".into(), true.into());
        out.insert(param.clone().into(), Yaml::Mapping(field));
    }
    out
}

fn fields_yaml(fields: &[itda_html::Field]) -> Yaml {
    Yaml::Sequence(
        fields
            .iter()
            .map(|field| {
                let mut out = Mapping::new();
                out.insert("selector".into(), field.selector.clone().into());
                if !field.attribute.is_empty() {
                    out.insert("attribute".into(), field.attribute.clone().into());
                }
                Yaml::Mapping(out)
            })
            .collect(),
    )
}
