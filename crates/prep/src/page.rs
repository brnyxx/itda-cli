//! Fetched pages to a draft definition.
//!
//! A page shows what it is made of: a table or a list of records, data the
//! server embedded for its scripts, forms, feeds and page links. Each becomes
//! an action with a response check, so `itda check` can tell when the page
//! changes. When the site also publishes an API description, its reads join
//! the draft. Pages and descriptions are fetched by the caller with the honest
//! client; nothing here reaches the network.
use crate::har::{Kind, action_name, plural, singular, site_name, template, unique};
use crate::label::{Label, describe, language, words};
use crate::openapi::openapi_parts;
use crate::{Described, Draft, Options, Parts, reorder, slug};
use itda_html::evidence;
use itda_types::error::{AppResult, usage};
use itda_types::util::secret_field;
use serde_json::Value;
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
    let (parts, described) = page_parts_with_description(pages, description, options)?;
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

/// The pages' parts with the description's reads and the GraphQL schemas'
/// reads merged in, before the final checks.
pub(crate) fn page_parts_with_description(
    pages: &[Page],
    description: Option<&Description>,
    options: &Options,
) -> AppResult<(Parts, Option<Described>)> {
    let mut parts = page_parts(pages, options)?;
    let described = description.and_then(|d| merge_description(&mut parts, d, options));
    let lang = language(&options.lang, &page_lang(&pages[0].html));
    crate::add_schemas(&mut parts, options, &lang);
    Ok((parts, described))
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
        session: None,
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
    let mut models = Mapping::new();
    let mut names = BTreeSet::new();
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
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
        // The same page seen twice is drafted once; two reads of one page
        // that return different embedded lists are both kept.
        let mut push = |entry: Mapping, notes: &mut Vec<String>| {
            let field = |name: &str| {
                entry
                    .get(name)
                    .and_then(Yaml::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let response = entry.get("response").and_then(Yaml::as_mapping);
            let list = response
                .map(|r| {
                    let text = |name: &str| r.get(name).and_then(Yaml::as_str).unwrap_or_default();
                    match text("ssr_json") {
                        "" => String::new(),
                        key => format!("{key} {}", text("extract_pointer")),
                    }
                })
                .unwrap_or_default();
            let key = (field("method"), field("path"), list);
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
        let listings = embedded_lists(&page.html);
        for entry in page_actions(
            page,
            &url,
            &path,
            &params,
            &mut names,
            &mut notes,
            &lang,
            !listings.is_empty(),
            (&origin, &prefix),
        ) {
            push(entry, &mut notes);
        }
        for listing in &listings {
            push(
                listing_action(
                    listing,
                    page,
                    &url,
                    &path,
                    &params,
                    &page_title,
                    &mut names,
                    &mut models,
                    &mut notes,
                    &lang,
                ),
                &mut notes,
            );
        }
        for form in evidence::forms(&page.html, url.as_str()) {
            let label = format!("{} form on {}", form.method, url.path());
            if form.password {
                sign_in = true;
                notes.push(if options.session.is_some() {
                    format!("{label}: the sign-in form; left out. `itda login {site}` signs in through it.")
                } else {
                    format!(
                        "{label}: a sign-in form; left out. Sign in with `itda login {site} --browser` once the definition is installed."
                    )
                });
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
    if let Some(session) = &options.session {
        // The pages were read signed in, so every action needs the session,
        // and the definition keeps how the sign-in went.
        let mut strategy = Mapping::new();
        strategy.insert("name".into(), "session".into());
        strategy.insert("kind".into(), "browser".into());
        if !session.cookie.is_empty() {
            strategy.insert(
                "value_from".into(),
                format!("cookie:{}", session.cookie).into(),
            );
        }
        let mut login = Mapping::new();
        login.insert("path".into(), session.login.path.clone().into());
        login.insert("user".into(), session.login.user.clone().into());
        login.insert("password".into(), session.login.password.clone().into());
        strategy.insert("login".into(), Yaml::Mapping(login));
        auth.push(Yaml::Mapping(strategy));
        for op in &mut operations {
            op.entry("auth".into()).or_insert_with(|| "session".into());
        }
    } else if sign_in {
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
        models,
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

/// The page itself as reads: its records, its own content when it is one
/// thing, its embedded data, or at least its title, so the action fails
/// when the page changes. A page that is one thing with a named list on
/// it (a package with its releases) gets both reads, and a list whose
/// links share one shape gets a read of one record by its id. A page
/// whose records live only in an embedded payload gets no read of its
/// own: the typed reads of those lists (`has_lists`) are its reads.
#[allow(clippy::too_many_arguments)]
fn page_actions(
    page: &Page,
    url: &url::Url,
    path: &str,
    params: &[String],
    names: &mut BTreeSet<String>,
    notes: &mut Vec<String>,
    lang: &str,
    has_lists: bool,
    site: (&url::Origin, &str),
) -> Vec<Mapping> {
    let title = evidence::title_field(&page.html);
    let page_title = title_of(&page.html);
    let extraction = evidence::infer_extraction(&page.html, url.as_str());
    // A page whose scripts are handed lists of records is a list page,
    // whatever its headline says.
    let main = if has_lists {
        None
    } else {
        evidence::main_content(&page.html, extraction.as_ref().map(|e| e.items.as_str()))
    };
    if has_lists && extraction.is_none() {
        return Vec::new();
    }
    let subject = if page_title.is_empty() {
        url.path().to_owned()
    } else {
        page_title.clone()
    };
    let label = format!("GET {}", url.path());
    let what = extraction.as_ref().and_then(|e| what_of_items(&e.items));
    let records: Vec<Value> = extraction
        .as_ref()
        .and_then(|e| e.extract(&page.html, url.as_str()))
        .and_then(|value| value.get("items").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let start = |name: &str, desc: &str, lists: bool| {
        let mut entry = Mapping::new();
        entry.insert("name".into(), name.into());
        entry.insert("desc".into(), desc.into());
        entry.insert("method".into(), "GET".into());
        entry.insert("path".into(), path.into());
        entry.insert("effect".into(), "read".into());
        page_inputs(&mut entry, page, url, params, lists);
        entry
    };
    let page_name = if path == "/" {
        "home".to_owned()
    } else {
        action_name("GET", path)
    };
    let list_read = |name: &str, extraction: &itda_html::Extraction| {
        let desc = match &what {
            Some(what) => Label::Records {
                what,
                title: &subject,
            },
            None => Label::List { title: &subject },
        };
        let mut entry = start(name, &describe(&desc, lang), true);
        let mut response = Mapping::new();
        response.insert("format".into(), "html".into());
        if let Some(title) = &title {
            response.insert(
                "required_html_fields".into(),
                fields_yaml(std::slice::from_ref(title)),
            );
        }
        response.insert("html".into(), extraction_yaml(extraction));
        entry.insert(
            "evidence".into(),
            format!(
                "{label} shows one record per `{}` with {} fields",
                extraction.items,
                extraction.fields.len()
            )
            .into(),
        );
        entry.insert("response".into(), Yaml::Mapping(response));
        entry
    };
    let mut out = Vec::new();
    let mut listed = false;
    if let Some(main) = &main {
        let name = if path == "/" {
            "home".to_owned()
        } else if path.ends_with('}') {
            action_name("GET", path)
        } else {
            let last = path
                .rsplit('/')
                .find(|s| !s.is_empty() && !s.starts_with('{'))
                .unwrap_or("page");
            slug(&format!("get-{}", last.split('.').next().unwrap_or(last)))
        };
        let name = unique(names, &name);
        let mut entry = start(
            &name,
            &describe(&Label::Page { title: &subject }, lang),
            false,
        );
        let mut response = Mapping::new();
        response.insert("format".into(), "html".into());
        response.insert(
            "required_html_fields".into(),
            fields_yaml(std::slice::from_ref(&main.headline)),
        );
        let mut html = Mapping::new();
        html.insert("page".into(), page_fields_yaml(&main.fields));
        response.insert("html".into(), Yaml::Mapping(html));
        entry.insert("response".into(), Yaml::Mapping(response));
        let mut parts = vec!["a headline".to_owned()];
        if main.body {
            parts.push("a body".into());
        }
        let other = main.fields.len() - 1 - usize::from(main.body) - main.values;
        if other > 0 {
            parts.push(format!("{other} more fields"));
        }
        if main.values > 0 {
            parts.push(format!("{} labelled values", main.values));
        }
        entry.insert(
            "evidence".into(),
            format!("{label} is one page with {}", parts.join(", ")).into(),
        );
        notes.push(format!(
            "{label}: reads the page's own content as {} fields; add html fields for anything else agents should read.",
            main.fields.len()
        ));
        out.push(entry);
        if let (Some(extraction), Some(what)) = (&extraction, &what)
            && records.len() >= 5
        {
            let name = unique(names, &slug(&format!("list-{what}")));
            out.push(list_read(&name, extraction));
            listed = true;
        }
    } else if let Some(extraction) = &extraction {
        let name = unique(names, &page_name);
        out.push(list_read(&name, extraction));
        listed = true;
    }
    if listed && let Some(detail) = detail_link(&records, site.0, site.1) {
        out.push(detail_action(
            &detail,
            what.as_deref(),
            &subject,
            &label,
            names,
            notes,
            lang,
        ));
    }
    if !out.is_empty() {
        return out;
    }

    let name = unique(names, &page_name);
    let mut entry = start(
        &name,
        &describe(&Label::Page { title: &subject }, lang),
        false,
    );
    let mut response = Mapping::new();
    response.insert("format".into(), "html".into());
    if !has_lists && let Some((key, pointer, pointers)) = embedded(&page.html) {
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
                "{label}: no table, list, embedded data or main content was found; only the title is checked. Add html fields for what agents should read."
            )
        });
    } else {
        entry.insert("evidence".into(), format!("{label} was fetched").into());
        notes.push(format!(
            "{label}: no title, records or embedded data were found; add a response check by hand."
        ));
        return vec![entry];
    }
    entry.insert("response".into(), Yaml::Mapping(response));
    vec![entry]
}

/// The page's path inputs and its query string as inputs with defaults, plus
/// a `page` input when the page links to its next page.
/// `lists` says the read returns records, so it can turn pages with the
/// key the page's links use; a read of a title or a whole payload has
/// nothing to go on to.
fn page_inputs(entry: &mut Mapping, page: &Page, url: &url::Url, params: &[String], lists: bool) {
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
    let mut pagination = None;
    if let Some(pages) = evidence::pagination(&page.html, url.as_str()) {
        if !query.contains_key(pages.key.as_str()) {
            let mut field = Mapping::new();
            field.insert("type".into(), "int".into());
            field.insert("default".into(), pages.default.into());
            query.insert(pages.key.clone().into(), Yaml::Mapping(field));
        }
        let counted = query
            .get(pages.key.as_str())
            .and_then(|field| field.get("type"))
            .and_then(Yaml::as_str)
            == Some("int");
        // A key that starts at 0 counts records, one that starts at 1 pages.
        let style = if pages.default == 0 { "offset" } else { "page" };
        if counted && lists {
            pagination = Some(crate::pagination_yaml(style, "", "", &pages.key, false));
        }
    }
    if !query.is_empty() {
        entry.insert("query".into(), Yaml::Mapping(query));
    }
    if let Some(pagination) = pagination {
        entry.insert("pagination".into(), pagination);
    }
}

/// Kinds of record a page lists, as the words sites name them with.
const KINDS: &[&str] = &[
    "story",
    "post",
    "article",
    "product",
    "question",
    "answer",
    "project",
    "event",
    "job",
    "recipe",
    "video",
    "book",
    "course",
    "lesson",
    "person",
    "place",
    "comment",
    "reply",
    "review",
    "release",
    "issue",
    "bug",
    "topic",
    "thread",
    "discussion",
    "notice",
    "package",
    "file",
    "version",
    "commit",
    "tag",
    "repository",
    "paper",
    "result",
    "message",
    "document",
    "photo",
    "image",
    "offer",
    "deal",
    "listing",
    "app",
    "episode",
    "track",
    "album",
    "movie",
    "game",
    "submission",
    "talk",
    "entry",
];

/// What the records a selector picks are, in plain words, from the names
/// the page gives them and their list: `ol.stories > li.story` lists
/// `stories`, `#articles > dt` lists `articles`, `tr.athing` says nothing.
fn what_of_items(items: &str) -> Option<String> {
    let steps: Vec<&str> = items.split(" > ").collect();
    let names = steps
        .into_iter()
        .rev()
        .flat_map(|step| step.split_whitespace())
        .filter_map(|step| {
            let step = step.split(':').next().unwrap_or(step);
            step.split_once('.')
                .map(|(_, class)| class)
                .or_else(|| step.strip_prefix('#'))
        });
    for name in names {
        let words = words(&slug(name));
        for word in words.split_whitespace().rev() {
            let one = singular(word);
            if KINDS.contains(&one.as_str()) {
                return Some(plural(&one));
            }
        }
    }
    None
}

/// Where the links of a list's records go: one path with one segment that
/// differs per record (`/item/{id}`), or one path with one query key that
/// does (`/item?id=`). Only when every link fits.
struct Detail {
    path: String,
    /// The query key that names the record, when the id is in the query.
    query: Option<String>,
    /// `int` or `string`, as the ids look.
    kind: &'static str,
    /// Query keys every link sends with the same value.
    constants: Vec<(String, String)>,
}

impl Detail {
    fn shape(&self) -> String {
        match &self.query {
            Some(key) => format!("{}?{key}=", self.path),
            None => self.path.clone(),
        }
    }
}

fn detail_link(records: &[Value], origin: &url::Origin, prefix: &str) -> Option<Detail> {
    let mut links: Vec<url::Url> = Vec::new();
    for record in records {
        let Some(link) = record.get("link").and_then(Value::as_str) else {
            continue;
        };
        let url = url::Url::parse(link).ok()?;
        if url.origin() != *origin || !url.path().starts_with(prefix) {
            return None;
        }
        if !links
            .iter()
            .any(|l| l.path() == url.path() && l.query() == url.query())
        {
            links.push(url);
        }
    }
    if links.len() < 3 {
        return None;
    }
    let kind_of = |values: &[String]| {
        if values.iter().all(|v| Kind::of_text(v) == Kind::Int) {
            "int"
        } else {
            "string"
        }
    };
    if links.iter().all(|l| l.query().is_none()) {
        let paths: Vec<Vec<&str>> = links
            .iter()
            .map(|l| l.path()[prefix.len()..].split('/').collect())
            .collect();
        let width = paths[0].len();
        if paths.iter().any(|p| p.len() != width) {
            return None;
        }
        let differing: Vec<usize> = (0..width)
            .filter(|&i| paths.iter().any(|p| p[i] != paths[0][i]))
            .collect();
        let [at] = differing.as_slice() else {
            return None;
        };
        let values: Vec<String> = paths.iter().map(|p| p[*at].to_owned()).collect();
        if values.iter().any(|v| v.is_empty() || secret_field(v)) {
            return None;
        }
        let path: Vec<&str> = paths[0]
            .iter()
            .enumerate()
            .map(|(i, s)| if i == *at { "{id}" } else { *s })
            .collect();
        return Some(Detail {
            path: path.join("/"),
            query: None,
            kind: kind_of(&values),
            constants: Vec::new(),
        });
    }
    let path = &links[0].path()[prefix.len()..];
    if links.iter().any(|l| &l.path()[prefix.len()..] != path) {
        return None;
    }
    let queries: Vec<Vec<(String, String)>> = links
        .iter()
        .map(|l| {
            l.query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .collect();
    let keys: Vec<&String> = queries[0].iter().map(|(k, _)| k).collect();
    if queries
        .iter()
        .any(|q| q.len() != keys.len() || q.iter().zip(&keys).any(|((k, _), key)| k != *key))
    {
        return None;
    }
    let mut varying = Vec::new();
    let mut constants = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        let values: Vec<String> = queries.iter().map(|q| q[i].1.clone()).collect();
        if values.iter().all(|v| *v == values[0]) {
            constants.push(((*key).clone(), values[0].clone()));
        } else {
            varying.push(((*key).clone(), values));
        }
    }
    let [(key, values)] = varying.as_slice() else {
        return None;
    };
    if secret_field(key) || values.iter().any(|v| v.is_empty()) {
        return None;
    }
    Some(Detail {
        path: path.to_owned(),
        query: Some(key.clone()),
        kind: kind_of(values),
        constants,
    })
}

/// A read of one record by its id, where the list's links go. No response
/// check yet: the page it opens was not fetched.
fn detail_action(
    detail: &Detail,
    what: Option<&str>,
    subject: &str,
    list_label: &str,
    names: &mut BTreeSet<String>,
    notes: &mut Vec<String>,
    lang: &str,
) -> Mapping {
    let one = what.map(singular).unwrap_or_else(|| {
        detail
            .path
            .split('/')
            .rfind(|s| !s.is_empty() && !s.starts_with('{'))
            .map(|s| singular(s.split('.').next().unwrap_or(s)))
            .unwrap_or_else(|| "item".into())
    });
    // The list of a board is `get-board`; one of its records is then
    // `get-board-by-id`, not `get-board-2`.
    let plain = slug(&format!("get-{one}"));
    let name = if names.contains(&plain) {
        unique(names, &format!("{plain}-by-id"))
    } else {
        unique(names, &plain)
    };
    let mut entry = Mapping::new();
    entry.insert("name".into(), name.into());
    entry.insert(
        "desc".into(),
        describe(
            &Label::One {
                what: what.unwrap_or("items"),
                title: subject,
            },
            lang,
        )
        .into(),
    );
    entry.insert("method".into(), "GET".into());
    entry.insert("path".into(), detail.path.clone().into());
    entry.insert("effect".into(), "read".into());
    entry.insert(
        "evidence".into(),
        format!("the records of {list_label} link to {}", detail.shape()).into(),
    );
    let mut id = Mapping::new();
    id.insert("type".into(), detail.kind.into());
    id.insert("required".into(), true.into());
    match &detail.query {
        None => {
            let mut params = Mapping::new();
            params.insert("id".into(), Yaml::Mapping(id));
            entry.insert("params".into(), Yaml::Mapping(params));
        }
        Some(key) => {
            let mut query = Mapping::new();
            query.insert(key.clone().into(), Yaml::Mapping(id));
            for (key, value) in &detail.constants {
                if secret_field(key) {
                    continue;
                }
                let kind = Kind::of_text(value);
                let mut field = Mapping::new();
                field.insert("type".into(), kind.name().into());
                field.insert(
                    "default".into(),
                    match kind {
                        Kind::Int => value
                            .parse::<i64>()
                            .map(Yaml::from)
                            .unwrap_or(value.as_str().into()),
                        Kind::Bool => (value == "true").into(),
                        _ => value.as_str().into(),
                    },
                );
                query.insert(key.clone().into(), Yaml::Mapping(field));
            }
            entry.insert("query".into(), Yaml::Mapping(query));
        }
    }
    notes.push(format!(
        "GET {}: reads one record where the list's links go, with no response check yet; run it once with an id, then declare what the page shows.",
        detail.shape()
    ));
    entry
}

/// Page-level fields as people write them: only what differs from the
/// defaults.
fn page_fields_yaml(fields: &std::collections::BTreeMap<String, itda_html::OutputField>) -> Yaml {
    let mut out = Mapping::new();
    for (name, field) in fields {
        let mut entry = Mapping::new();
        entry.insert("selector".into(), field.selector.clone().into());
        if !field.attribute.is_empty() {
            entry.insert("attribute".into(), field.attribute.clone().into());
        }
        if field.required {
            entry.insert("required".into(), true.into());
        }
        out.insert(name.clone().into(), Yaml::Mapping(entry));
    }
    Yaml::Mapping(out)
}

/// One list of records inside an embedded payload: where it is, what it
/// holds in plain words, and the fields a model keeps from each record.
struct Listing {
    /// The payload key: `ld+json`, `__NEXT_DATA__`, `__APOLLO_STATE__`...
    key: String,
    /// JSON pointer to the array inside the payload.
    pointer: String,
    /// What the records are, in plain words: `news articles`, `products`.
    what: String,
    /// `(field, path, type)` in the order a model lists them.
    fields: Vec<(String, String, &'static str)>,
    count: usize,
}

/// A typed read of one embedded list: the page fetched again, the list
/// taken out with `extract_pointer` and each record cut down by a model.
#[allow(clippy::too_many_arguments)]
fn listing_action(
    listing: &Listing,
    page: &Page,
    url: &url::Url,
    path: &str,
    params: &[String],
    page_title: &str,
    names: &mut BTreeSet<String>,
    models: &mut Mapping,
    notes: &mut Vec<String>,
    lang: &str,
) -> Mapping {
    let what = &listing.what;
    let name = unique(names, &slug(&format!("list-{what}")));
    let subject = if page_title.is_empty() {
        url.path().to_owned()
    } else {
        page_title.to_owned()
    };
    let mut entry = Mapping::new();
    entry.insert("name".into(), name.into());
    entry.insert(
        "desc".into(),
        describe(
            &Label::Records {
                what,
                title: &subject,
            },
            lang,
        )
        .into(),
    );
    entry.insert("method".into(), "GET".into());
    entry.insert("path".into(), path.into());
    entry.insert("effect".into(), "read".into());
    let label = format!("GET {}", url.path());
    entry.insert(
        "evidence".into(),
        format!(
            "{label} embeds {} {what} in its {} script at {}",
            listing.count, listing.key, listing.pointer
        )
        .into(),
    );
    page_inputs(&mut entry, page, url, params, true);
    let mut response = Mapping::new();
    response.insert("format".into(), "html".into());
    response.insert("ssr_json".into(), listing.key.clone().into());
    response.insert("extract_pointer".into(), listing.pointer.clone().into());
    // The fields that make a record one of these must be in the first one,
    // so a page that puts something else at the pointer fails the check.
    // One field per role: the title, then the id, then the link, each the
    // first (shallowest) the model lists.
    let required: Vec<Yaml> = [Role::Title, Role::Id, Role::Url]
        .into_iter()
        .filter_map(|wanted| {
            listing
                .fields
                .iter()
                .find(|(_, path, _)| role(leaf(path)) == wanted)
        })
        .map(|(_, path, _)| format!("/0/{}", path.replace('.', "/")).into())
        .collect();
    if !required.is_empty() {
        response.insert("required_pointers".into(), Yaml::Sequence(required));
    }
    entry.insert("response".into(), Yaml::Mapping(response));

    let mut model = Mapping::new();
    for (field, path, kind) in &listing.fields {
        let mut spec = Mapping::new();
        if path != field {
            spec.insert("path".into(), path.clone().into());
        }
        spec.insert("type".into(), (*kind).into());
        model.insert(field.clone().into(), Yaml::Mapping(spec));
    }
    let model_name = model_name(models, &slug(&singular(what)), Yaml::Mapping(model));
    entry.insert("model".into(), model_name.into());
    notes.push(format!(
        "{label}: the {what} come from the {} payload at {}; the `{}` model keeps {} fields of each, add or rename them as agents need.",
        listing.key,
        listing.pointer,
        entry
            .get("model")
            .and_then(Yaml::as_str)
            .unwrap_or_default(),
        listing.fields.len()
    ));
    entry
}

/// The name a model is kept under: the same name when an equal model is
/// already there, a numbered one when a different model has the name.
fn model_name(models: &mut Mapping, base: &str, model: Yaml) -> String {
    let base = if base.is_empty() { "record" } else { base };
    let mut candidate = base.to_owned();
    let mut n = 2;
    while let Some(existing) = models.get(candidate.as_str()) {
        if *existing == model {
            return candidate;
        }
        candidate = format!("{base}-{n}");
        n += 1;
    }
    models.insert(candidate.clone().into(), model);
    candidate
}

/// What a record field is for, in the order a model lists them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Id,
    Title,
    Url,
    Date,
    Summary,
    Other,
}

fn role(key: &str) -> Role {
    let lower = key.to_ascii_lowercase();
    let plain = lower.replace(['-', '_'], "");
    if plain == "id"
        || plain == "slug"
        || plain == "urlslug"
        || (plain.ends_with("id") && plain.len() > 2)
    {
        Role::Id
    } else if matches!(
        plain.as_str(),
        "title" | "headline" | "name" | "subject" | "label" | "caption"
    ) {
        Role::Title
    } else if matches!(plain.as_str(), "url" | "href" | "link" | "permalink")
        || plain.ends_with("url")
        || plain.ends_with("link")
    {
        Role::Url
    } else if plain.contains("date")
        || plain.contains("time")
        || plain.ends_with("at")
        || plain.starts_with("published")
        || plain.starts_with("created")
        || plain.starts_with("updated")
        || plain.starts_with("modified")
    {
        Role::Date
    } else if matches!(
        plain.as_str(),
        "description"
            | "summary"
            | "excerpt"
            | "shortdescription"
            | "body"
            | "content"
            | "text"
            | "abstract"
            | "snippet"
            | "articlebody"
    ) {
        Role::Summary
    } else {
        Role::Other
    }
}

/// Words in a pointer that mark a list of links around the page, not the
/// page's records.
const NAVIGATION: &[&str] = &[
    "navigation",
    "nav",
    "menu",
    "footer",
    "header",
    "sidebar",
    "breadcrumb",
    "legal",
];

/// The last name in a model path: `issue.title` is `title`.
fn leaf(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

/// Payload keys whose arrays say nothing about what the page lists.
const GENERIC: &[&str] = &[
    "data",
    "items",
    "list",
    "results",
    "result",
    "edges",
    "nodes",
    "entries",
    "props",
    "pageprops",
    "page",
    "state",
    "initialdata",
    "hydrated",
    "payload",
    "records",
    "rows",
    "elements",
    "itemlistelement",
    "blocks",
];

/// Does the page embed lists of records for its scripts? Such a page is a
/// list page, and gets typed reads of those lists rather than a read of
/// its own content.
pub fn embeds_lists(html: &str) -> bool {
    !embedded_lists(html).is_empty()
}

/// Lists of records in the page's embedded payloads, each with the fields
/// a model should keep: in ld+json an `ItemList` or an array of one typed
/// thing (`NewsArticle`, `Product`...); in the other payloads the array of
/// objects with the most to read (a title, a link, a date...) and, among
/// those, the longest.
fn embedded_lists(html: &str) -> Vec<Listing> {
    const MAX_LISTS: usize = 4;
    let map = itda_html::extract_ssr_map(html);
    let mut out = Vec::new();
    if let Some(root) = map.get("ld+json") {
        out.extend(ld_json_lists(root));
    }
    let keys = SSR_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .chain(map.keys().cloned())
        .filter(|key| key != "ld+json");
    let mut done = BTreeSet::new();
    for key in keys {
        if out.len() >= MAX_LISTS || !done.insert(key.clone()) {
            continue;
        }
        let Some(root) = map.get(&key) else {
            continue;
        };
        let mut lists = Vec::new();
        lists_in(root, "", 0, &mut lists);
        // Menus, footers and the like are not what the page lists.
        lists.retain(|(pointer, _)| {
            !pointer.split('/').any(|segment| {
                let lower = segment.to_ascii_lowercase();
                NAVIGATION.iter().any(|word| lower.contains(word))
            })
        });
        let scored: Vec<_> = lists
            .iter()
            .map(|(pointer, records)| {
                let fields = fields_of(records, "");
                let roles: BTreeSet<Role> = fields
                    .iter()
                    .map(|(_, path, _)| role(leaf(path)))
                    .filter(|r| *r != Role::Other)
                    .collect();
                let titled = roles.contains(&Role::Title) && roles.len() >= 2;
                // A list under a name (`/Question`) over the same records
                // under a generic one (`/items`).
                let named = pointer
                    .rsplit('/')
                    .next()
                    .is_some_and(|last| !GENERIC.contains(&last.to_ascii_lowercase().as_str()));
                (
                    titled,
                    roles.len(),
                    records.len(),
                    named,
                    fields,
                    pointer,
                    records,
                )
            })
            .collect();
        // A list with a title and something else to read wins; without one,
        // the longest list, as drafts did before.
        let best = scored
            .iter()
            .filter(|(titled, ..)| *titled)
            .max_by_key(|(_, roles, len, named, ..)| (*roles, *len, *named))
            .or_else(|| scored.iter().max_by_key(|(_, _, len, ..)| *len));
        if let Some((_, _, _, _, fields, pointer, records)) = best
            && !fields.is_empty()
        {
            let (fields, pointer, records) = (fields.clone(), *pointer, *records);
            let what = what_of(pointer, records);
            out.push(Listing {
                key: key.clone(),
                pointer: pointer.clone(),
                what,
                fields,
                count: records.len(),
            });
        }
    }
    out.truncate(MAX_LISTS);
    out
}

/// ld+json types that describe the site or the page, never a list of things.
const BOILERPLATE: &[&str] = &[
    "Organization",
    "WebSite",
    "WebPage",
    "BreadcrumbList",
    "ListItem",
    "SearchAction",
    "ImageObject",
    "Brand",
    "SiteNavigationElement",
];

fn ld_json_lists(root: &Value) -> Vec<Listing> {
    let mut out = Vec::new();
    ld_json_walk(root, "", 0, &mut out);
    out
}

fn ld_json_walk(value: &Value, pointer: &str, depth: usize, out: &mut Vec<Listing>) {
    if depth > 8 || out.len() >= 3 {
        return;
    }
    match value {
        Value::Object(object) => {
            let kind = object
                .get("@type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if kind == "ItemList"
                && let Some(elements) = object.get("itemListElement").and_then(Value::as_array)
                && elements.len() >= 3
                && elements.iter().all(Value::is_object)
            {
                let pointer = format!("{pointer}/itemListElement");
                let nested = elements
                    .iter()
                    .all(|e| e.get("item").is_some_and(Value::is_object));
                let (fields, item_type) = if nested {
                    let items: Vec<Value> = elements
                        .iter()
                        .filter_map(|e| e.get("item").cloned())
                        .collect();
                    (fields_of(&items, "item."), ld_type(&items))
                } else {
                    (fields_of(elements, ""), String::new())
                };
                let what = if !item_type.is_empty() {
                    plural(&words(&item_type))
                } else {
                    object
                        .get("name")
                        .and_then(Value::as_str)
                        .map(|name| words(&slug(name)))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "items".into())
                };
                if !fields.is_empty() {
                    out.push(Listing {
                        key: "ld+json".into(),
                        pointer,
                        what,
                        fields,
                        count: elements.len(),
                    });
                }
                return;
            }
            for (name, child) in object.iter().take(100) {
                ld_json_walk(
                    child,
                    &format!("{pointer}/{}", escape(name)),
                    depth + 1,
                    out,
                );
            }
        }
        Value::Array(items) => {
            let kind = ld_type(items);
            if items.len() >= 3 && !kind.is_empty() && !BOILERPLATE.contains(&kind.as_str()) {
                let fields = fields_of(items, "");
                if !fields.is_empty() {
                    out.push(Listing {
                        key: "ld+json".into(),
                        pointer: pointer.to_owned(),
                        what: plural(&words(&kind)),
                        fields,
                        count: items.len(),
                    });
                    return;
                }
            }
            for (index, item) in items.iter().enumerate().take(50) {
                ld_json_walk(item, &format!("{pointer}/{index}"), depth + 1, out);
            }
        }
        _ => {}
    }
}

/// The one `@type` every object in the list has, or nothing.
fn ld_type(items: &[Value]) -> String {
    let mut kinds = items.iter().map(|item| {
        item.get("@type")
            .and_then(|t| t.as_str().or_else(|| t.as_array()?.first()?.as_str()))
            .unwrap_or_default()
    });
    let first = kinds.next().unwrap_or_default();
    if first.is_empty() || kinds.any(|k| k != first) {
        String::new()
    } else {
        first.to_owned()
    }
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Every list of at least three records (objects) in the payload, with its
/// JSON pointer.
fn lists_in<'a>(
    value: &'a Value,
    pointer: &str,
    depth: usize,
    out: &mut Vec<(String, &'a Vec<Value>)>,
) {
    if depth > 12 || out.len() >= 400 {
        return;
    }
    match value {
        Value::Array(items) => {
            if items.len() >= 3 && items.iter().all(Value::is_object) {
                out.push((pointer.to_owned(), items));
            }
            for (index, item) in items.iter().enumerate().take(50) {
                lists_in(item, &format!("{pointer}/{index}"), depth + 1, out);
            }
        }
        Value::Object(object) => {
            for (name, child) in object.iter().take(200) {
                lists_in(
                    child,
                    &format!("{pointer}/{}", escape(name)),
                    depth + 1,
                    out,
                );
            }
        }
        _ => {}
    }
}

/// The fields a model keeps from these records: the plain values every
/// record has, at the top or one object down (`issue.title`), ids and
/// titles first, links and dates next, at most eight, never one that looks
/// like a credential. `prefix` puts them all under a nested object
/// (`item.`).
fn fields_of(records: &[Value], prefix: &str) -> Vec<(String, String, &'static str)> {
    const MAX_FIELDS: usize = 8;
    let mut fields: Vec<(Role, String, String, &'static str)> = Vec::new();
    for (key, kind) in plain_fields(records) {
        fields.push((role(&key), key.clone(), format!("{prefix}{key}"), kind));
    }
    for key in shared_keys(records) {
        let nested: Vec<Value> = records
            .iter()
            .filter_map(|record| record.get(&key))
            .filter(|value| value.is_object())
            .cloned()
            .collect();
        if nested.len() != records.len() {
            continue;
        }
        for (name, kind) in plain_fields(&nested) {
            fields.push((
                role(&name),
                name.clone(),
                format!("{prefix}{key}.{name}"),
                kind,
            ));
        }
    }
    // `id` before `post_id`, `title` before `name`; two ids are enough.
    let rank = |key: &str| match key.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "id" | "title" | "url" => 0,
        "slug" | "urlslug" | "headline" | "href" | "link" => 1,
        _ => 2,
    };
    // The record's own fields first, then what sits one object down.
    let depth = |path: &str| path.trim_start_matches(prefix).matches('.').count();
    fields.sort_by(|a, b| {
        depth(&a.2)
            .cmp(&depth(&b.2))
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| rank(&a.1).cmp(&rank(&b.1)))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut ids = 0;
    fields.retain(|(role, _, _, _)| {
        if *role != Role::Id {
            return true;
        }
        ids += 1;
        ids <= 2
    });
    // A nested field keeps its own name (`issue.title` is `title`) unless
    // the record already has a field in that role or of that name: then
    // `author.name` is `author_name` and `issue.id` next to `id` is `issue_id`.
    let top_roles: BTreeSet<Role> = fields
        .iter()
        .filter(|(_, _, path, _)| !path.trim_start_matches(prefix).contains('.'))
        .map(|(role, ..)| *role)
        .collect();
    let mut taken = BTreeSet::new();
    fields
        .into_iter()
        .take(MAX_FIELDS)
        .map(|(role, name, path, kind)| {
            let inner = path.trim_start_matches(prefix);
            let nested = inner.contains('.');
            let shadowed = nested && role < Role::Date && top_roles.contains(&role);
            let field = if !shadowed && taken.insert(name.clone()) {
                name
            } else {
                let long = inner.replace('.', "_");
                taken.insert(long.clone());
                long
            };
            (field, path, kind)
        })
        .collect()
}

/// Keys every record has.
fn shared_keys(records: &[Value]) -> BTreeSet<String> {
    records
        .iter()
        .filter_map(Value::as_object)
        .map(|object| object.keys().cloned().collect::<BTreeSet<_>>())
        .reduce(|a, b| a.intersection(&b).cloned().collect())
        .unwrap_or_default()
        .into_iter()
        .filter(|key| {
            !key.is_empty()
                && !key.starts_with('@')
                && !key.starts_with("__")
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                && !secret_field(key)
        })
        .collect()
}

/// The shared keys whose values are strings, numbers or booleans, with the
/// type of the first value that is not null.
fn plain_fields(records: &[Value]) -> Vec<(String, &'static str)> {
    shared_keys(records)
        .into_iter()
        .filter_map(|key| {
            let value = records
                .iter()
                .filter_map(|record| record.get(&key))
                .find(|value| !value.is_null())?;
            value_is_plain(value).then(|| (key, Kind::of_json(value).name()))
        })
        .collect()
}

fn value_is_plain(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

/// What the records at `pointer` are, from the key that holds them or,
/// when that says nothing (`data`, `items`), from their own type.
fn what_of(pointer: &str, records: &[Value]) -> String {
    // A type every record names the same way: `__typename`, `@type`,
    // `itemType: "issue"`.
    let typename = [
        "__typename",
        "@type",
        "itemType",
        "item_type",
        "type",
        "kind",
    ]
    .into_iter()
    .find_map(|key| {
        let mut values = records.iter().map(|r| r.get(key).and_then(Value::as_str));
        let first = values.next().flatten()?;
        (!first.is_empty() && values.all(|v| v == Some(first))).then(|| first.to_owned())
    })
    .map(|t| {
        // A constant such as `REPO_ITEM_TYPE_ISSUES` says it last.
        let constant = t
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit());
        let t = if constant {
            t.rsplit('_').next().unwrap_or(&t).to_owned()
        } else {
            t
        };
        plural(&words(&slug(&t)))
    })
    .filter(|t| !t.is_empty());
    let from_key = pointer
        .split('/')
        .rev()
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .filter(|segment| !segment.is_empty() && segment.parse::<u64>().is_err())
        .map(|segment| words(&slug(&segment)))
        .find(|segment| {
            !segment.is_empty() && !GENERIC.contains(&segment.replace(' ', "").as_str())
        })
        .map(|segment| plural(&segment));
    let what = typename.or(from_key).unwrap_or_else(|| "items".into());
    if slug(&what).is_empty() {
        "items".into()
    } else {
        what
    }
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
