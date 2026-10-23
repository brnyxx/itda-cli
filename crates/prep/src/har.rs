//! Browser recordings (HAR files) to a draft definition.
//!
//! A recording holds real sign-in headers, cookies and personal data, so only
//! the shape of each call is kept: method, path, input names and types, and the
//! response fields every sample had. No header, cookie, query or body value is
//! copied into the draft.
use crate::label::{Label, describe, language};
use crate::{Draft, Options, Parts, reorder, slug};
use itda_types::error::{AppResult, usage};
use itda_types::util::secret_field;
use serde_json::Value;
use serde_norway::{Mapping, Value as Yaml};
use std::collections::{BTreeMap, BTreeSet};

const MAX_OPERATIONS: usize = 200;
/// Request headers that carry a token the site's API needs. Only the header
/// name reaches the draft; the token itself goes in the keyring later.
const TOKEN_HEADERS: &[&str] = &["authorization", "x-api-key", "api-key", "x-auth-token"];

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Int,
    Float,
    Bool,
    Str,
}

impl Kind {
    pub(crate) fn of_json(value: &Value) -> Kind {
        Kind::of(value)
    }

    fn of(value: &Value) -> Kind {
        match value {
            Value::Bool(_) => Kind::Bool,
            Value::Number(n) if n.is_i64() || n.is_u64() => Kind::Int,
            Value::Number(_) => Kind::Float,
            Value::String(s) => Kind::of_text(s),
            _ => Kind::Str,
        }
    }

    pub(crate) fn of_text(text: &str) -> Kind {
        if text.parse::<i64>().is_ok() && !(text.len() > 1 && text.starts_with('0')) {
            Kind::Int
        } else if text == "true" || text == "false" {
            Kind::Bool
        } else {
            Kind::Str
        }
    }

    fn merge(self, other: Kind) -> Kind {
        match (self, other) {
            (a, b) if a == b => a,
            (Kind::Int, Kind::Float) | (Kind::Float, Kind::Int) => Kind::Float,
            _ => Kind::Str,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Int => "int",
            Kind::Float => "float",
            Kind::Bool => "bool",
            Kind::Str => "string",
        }
    }
}

/// Input names seen across samples, with how often and as what.
#[derive(Default)]
struct Fields(BTreeMap<String, (usize, Kind)>);

impl Fields {
    fn add(&mut self, name: &str, kind: Kind) {
        self.0
            .entry(name.to_owned())
            .and_modify(|(count, k)| {
                *count += 1;
                *k = k.merge(kind);
            })
            .or_insert((1, kind));
    }

    /// A field is required when every sample sent it.
    fn yaml(&self, samples: usize) -> Mapping {
        let mut out = Mapping::new();
        for (name, (count, kind)) in &self.0 {
            let mut field = Mapping::new();
            field.insert("type".into(), kind.name().into());
            if *count == samples {
                field.insert("required".into(), true.into());
            }
            out.insert(name.clone().into(), Yaml::Mapping(field));
        }
        out
    }
}

/// Names of cookies in a `Cookie` header that look like a sign-in. Only names
/// are read; values never leave this function.
fn session_cookies(header: &str) -> BTreeSet<String> {
    header
        .split(';')
        .filter_map(|pair| pair.split_once('=').map(|(name, _)| name.trim()))
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            ["sess", "sid", "auth", "token", "login", "jwt", "remember"]
                .iter()
                .any(|hint| lower.contains(hint))
        })
        .map(str::to_owned)
        .collect()
}

/// Every recorded request with the same method and path shape.
#[derive(Default)]
struct Call {
    method: String,
    path: String,
    params: Vec<String>,
    samples: usize,
    query: Fields,
    json: Fields,
    form: Fields,
    multipart: bool,
    /// Text parts of a multipart body, and the name of the part that
    /// carried a file.
    parts: Fields,
    file_part: Option<String>,
    /// The media types of answers the site sent as attachments.
    attachments: BTreeSet<String>,
    left_out: BTreeSet<String>,
    /// Top-level response fields present in every JSON sample.
    response_keys: Option<BTreeSet<String>>,
    response_json: bool,
    token: Option<(String, bool)>,
    /// Sign-in cookie names every sample sent; `None` before the first sample.
    session: Option<BTreeSet<String>>,
    /// How many samples carried the site's session in the query.
    query_session: usize,
    /// A sample was the sign-in itself: a post with a password, or to a
    /// sign-in path. A person signs in, never an agent.
    sign_in: bool,
    /// Form fields that carried a token, such as `_csrf`.
    form_tokens: BTreeSet<String>,
    /// A header that carried an anti-forgery token, such as `X-CSRF-Token`.
    header_token: Option<String>,
    /// Position of the first sample in the recording.
    first: Option<usize>,
    /// The GraphQL operation these samples sent, when the body was one.
    graphql: Option<Graphql>,
    /// The `data` field under which every GraphQL answer had a
    /// `pageInfo.endCursor`, and whether `hasNextPage` came with it.
    page_info: Option<(String, bool)>,
}

/// One GraphQL operation: a query is a read, a mutation a change, and its
/// variables are the inputs. The document itself is kept; it is the
/// request's shape, not a value the person typed.
#[derive(Default)]
struct Graphql {
    name: String,
    document: String,
    mutation: bool,
    variables: Fields,
    /// Variables that held objects or lists, which inputs cannot express.
    complex: BTreeSet<String>,
}

/// The operation a JSON body sends, when it is a GraphQL document.
pub(crate) fn graphql_operation(post: &Value) -> Option<(String, String, bool)> {
    use graphql_parser::query::{Definition, OperationDefinition};
    if !post["mimeType"]
        .as_str()
        .is_some_and(|m| m.contains("json"))
    {
        return None;
    }
    let body: Value = serde_json::from_str(post["text"].as_str()?).ok()?;
    let document = body["query"].as_str()?;
    let parsed = graphql_parser::parse_query::<&str>(document).ok()?;
    let mut name = body["operationName"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut mutation = false;
    for definition in &parsed.definitions {
        let Definition::Operation(operation) = definition else {
            continue;
        };
        let found = match operation {
            OperationDefinition::Query(q) => q.name,
            OperationDefinition::Mutation(m) => {
                mutation = true;
                m.name
            }
            OperationDefinition::Subscription(s) => {
                mutation = true;
                s.name
            }
            OperationDefinition::SelectionSet(_) => None,
        };
        if name.is_empty()
            && let Some(found) = found
        {
            name = found.to_owned();
        }
    }
    Some((name, document.to_owned(), mutation))
}

pub fn from_har(text: &[u8], options: &Options) -> AppResult<Draft> {
    let mut parts = har_parts(text, options)?;
    crate::add_schemas(&mut parts, options, &language(&options.lang, ""));
    if parts.operations.is_empty() {
        return Err(no_operations());
    }
    Ok(parts.finish())
}

pub(crate) fn no_operations() -> itda_types::error::AppError {
    usage(
        "Could not draft any operations",
        "Check that the recording has API calls on this address, or loosen --include.",
    )
}

/// The calls in a recording as operations, without the final checks; a
/// recording of only pages gives none.
pub(crate) fn har_parts(text: &[u8], options: &Options) -> AppResult<Parts> {
    let doc: Value = serde_json::from_slice(text).map_err(|_| not_har())?;
    let entries = doc["log"]["entries"].as_array().ok_or_else(not_har)?;
    let located = |keep: &dyn Fn(&Value) -> bool| -> Vec<(&Value, url::Url)> {
        entries
            .iter()
            .filter(|entry| keep(entry))
            .filter_map(|entry| {
                Some((
                    entry,
                    url::Url::parse(entry["request"]["url"].as_str()?).ok()?,
                ))
            })
            .filter(|(_, url)| matches!(url.scheme(), "http" | "https"))
            .collect()
    };
    let recorded = located(&api_call);
    // The site is where the person's pages came from; when its API lives
    // elsewhere (a search service, say), the busiest API address is tried
    // next, and telemetry sent elsewhere never wins by count alone.
    let candidates: Vec<url::Url> = match &options.base_url {
        Some(base) => vec![url::Url::parse(base.trim_end_matches('/')).map_err(|_| {
            usage(
                "--base-url is not a URL",
                "Pass an address like https://example.com/api.",
            )
        })?],
        None => {
            let pages = located(&html_page);
            let mut candidates = Vec::new();
            for base in [busiest_origin(&pages), busiest_origin(&recorded)]
                .into_iter()
                .flatten()
            {
                if !candidates.contains(&base) {
                    candidates.push(base);
                }
            }
            candidates
        }
    };
    if candidates.is_empty() {
        return Err(usage(
            "The recording has no calls",
            "Record again with the browser's network panel open while you use the site, then save it as HAR with content.",
        ));
    }
    let mut first = None;
    for base in candidates {
        let parts = parts_at(entries, &recorded, &base, options)?;
        if !parts.operations.is_empty() {
            return Ok(parts);
        }
        first.get_or_insert(parts);
    }
    first.ok_or_else(no_operations)
}

/// The operations under one base address.
fn parts_at(
    entries: &[Value],
    recorded: &[(&Value, url::Url)],
    base: &url::Url,
    options: &Options,
) -> AppResult<Parts> {
    let origin = base.origin();
    let prefix = base.path().trim_end_matches('/').to_owned();
    let base_url = format!("{}{prefix}", origin.ascii_serialization());

    let mut notes = Vec::new();
    let mut elsewhere = BTreeSet::new();
    let mut calls: BTreeMap<(String, String, String), Call> = BTreeMap::new();
    let mut cookies = false;
    let query_session = query_session_key(entries);
    for (entry, url) in recorded {
        let Some(rest) = url
            .path()
            .strip_prefix(&prefix)
            .filter(|_| url.origin() == origin)
        else {
            elsewhere.insert(url.host_str().unwrap_or_default().to_owned());
            continue;
        };
        let request = &entry["request"];
        let method = request["method"]
            .as_str()
            .unwrap_or("GET")
            .to_ascii_uppercase();
        let (path, params) = template(rest);
        if let Some(filter) = &options.include
            && !path.to_lowercase().contains(&filter.to_lowercase())
        {
            continue;
        }
        let graphql = graphql_operation(&request["postData"]);
        let operation = graphql
            .as_ref()
            .map(|(name, _, _)| name.clone())
            .unwrap_or_default();
        let call = calls
            .entry((path.clone(), method.clone(), operation))
            .or_default();
        if let Some((name, document, mutation)) = graphql {
            let entry = call.graphql.get_or_insert_with(Graphql::default);
            entry.name = name;
            entry.document = document;
            entry.mutation |= mutation;
        }
        call.first.get_or_insert_with(|| {
            entries
                .iter()
                .position(|e| std::ptr::eq(e, *entry))
                .unwrap_or(0)
        });
        call.sign_in |= method == "POST" && sign_in_call(entry);
        call.method = method;
        call.path = path;
        call.params = params;
        call.samples += 1;
        for (name, value) in url.query_pairs() {
            if name == "_" {
                continue;
            }
            // The session is auth, kept in the keyring; never an input.
            if query_session.as_deref() == Some(name.as_ref()) {
                call.query_session += 1;
                continue;
            }
            if secret_field(&name) {
                call.left_out.insert(name.into_owned());
            } else {
                call.query.add(&name, Kind::of_text(&value));
            }
        }
        record_body(call, &request["postData"]);
        let mut sent = BTreeSet::new();
        for header in request["headers"].as_array().into_iter().flatten() {
            let name = header["name"].as_str().unwrap_or_default();
            let lower = name.to_ascii_lowercase();
            if lower == "cookie" {
                cookies = true;
                sent.extend(session_cookies(
                    header["value"].as_str().unwrap_or_default(),
                ));
            } else if TOKEN_HEADERS.contains(&lower.as_str()) {
                let bearer = header["value"]
                    .as_str()
                    .is_some_and(|v| v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer "));
                call.token = Some((name.to_owned(), bearer));
            } else if lower.contains("csrf") || lower.contains("xsrf") {
                call.header_token = Some(name.to_owned());
            }
        }
        call.session = Some(match call.session.take() {
            Some(before) => before.intersection(&sent).cloned().collect(),
            None => sent,
        });
        record_response(call, &entry["response"]["content"]);
        if let Some(media) = attachment_answer(entry) {
            call.attachments.insert(media);
        }
    }
    if !elsewhere.is_empty() {
        notes.push(format!(
            "Left out calls to other addresses: {}.",
            elsewhere.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let site = options
        .name
        .as_deref()
        .map(slug)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| site_name(base.host_str().unwrap_or_default()));
    // Calls that sent a sign-in cookie and no token use the browser session.
    let signed_in: Vec<&BTreeSet<String>> = calls
        .values()
        .filter(|call| call.token.is_none())
        .filter_map(|call| call.session.as_ref().filter(|names| !names.is_empty()))
        .collect();
    let sign_in_cookie = signed_in
        .first()
        .and_then(|first| {
            first
                .iter()
                .find(|name| signed_in.iter().all(|names| names.contains(*name)))
        })
        .cloned();
    let query_signed_in = calls.values().any(|call| call.query_session > 0);
    let login = query_signed_in
        .then(|| login_block(entries, &origin, &prefix))
        .flatten();
    let storage_signed_in = options.storage.as_ref().is_some_and(|storage| {
        calls.values().any(|call| {
            call.token
                .as_ref()
                .is_some_and(|(header, _)| storage.header.eq_ignore_ascii_case(header))
        })
    });
    if storage_signed_in && let Some(storage) = &options.storage {
        notes.push(format!(
            "Actions with auth: session send a bearer token the site keeps in the browser ({key} in its storage). It is kept in the keyring, never in the definition. Sign in again with `itda login {site} --browser` once after installing.",
            key = storage.key
        ));
    } else if query_signed_in && let Some(key) = &query_session {
        notes.push(match &login {
            Some(_) => format!(
                "Actions with auth: session carry the {key} query parameter the site hands out at sign-in. It is kept in the keyring, never in the definition. Run `itda login {site}` once after installing."
            ),
            None => format!(
                "Actions with auth: session carry the {key} query parameter the site hands out at sign-in. It is kept in the keyring, never in the definition. Add the sign-in form to the auth entry as `login`, then run `itda login {site}`."
            ),
        });
    } else if !signed_in.is_empty() {
        notes.push(format!(
            "Actions with auth: session use your browser sign-in; run `itda login {site} --browser` once after installing."
        ));
    } else if cookies {
        notes.push(
            "The site's cookies do not look like a sign-in; if actions report auth_required, add an auth entry with kind: browser."
                .into(),
        );
    }

    let mut auth: Vec<Yaml> = Vec::new();
    let mut auth_names: BTreeMap<String, String> = BTreeMap::new();
    let mut operations = Vec::new();
    let mut names = BTreeSet::new();
    for call in calls.values() {
        if operations.len() == MAX_OPERATIONS {
            notes.push(format!(
                "Stopped at {MAX_OPERATIONS} operations; narrow the draft with --include."
            ));
            break;
        }
        let label = format!("{} {}", call.method, call.path);
        if call.method == "POST" && telemetry(&call.path) {
            notes.push(format!(
                "{label}: left out; it looks like telemetry, not something an agent should send."
            ));
            continue;
        }
        if call.multipart && call.file_part.is_none() && call.parts.0.is_empty() {
            notes.push(format!(
                "{label}: skipped; the recording keeps no field names for this upload. Add body: {{multipart: {{file: {{name: <field>, from: file}}}}}} by hand."
            ));
            continue;
        }
        if call.sign_in && call.token.is_none() {
            notes.push(format!(
                "{label}: the sign-in; left out. `itda login {site}` signs in through the site's form."
            ));
            continue;
        }
        let name = match &call.graphql {
            Some(graphql) if !slug(&graphql.name).is_empty() => {
                unique(&mut names, &slug(&graphql.name))
            }
            Some(_) => unique(&mut names, "graphql"),
            None => unique(&mut names, &action_name(&call.method, &call.path)),
        };
        let mut entry = Mapping::new();
        entry.insert("name".into(), name.clone().into());
        let lang = language(&options.lang, "");
        let desc = match &call.graphql {
            Some(graphql) => describe(
                &Label::Graphql {
                    name: if graphql.name.is_empty() {
                        &name
                    } else {
                        &graphql.name
                    },
                    mutation: graphql.mutation,
                },
                &lang,
            ),
            None => {
                let (verb, resource) = name.split_once('-').unwrap_or(("get", &name));
                describe(&Label::Api { verb, resource }, &lang)
            }
        };
        entry.insert("desc".into(), desc.into());
        entry.insert("method".into(), call.method.clone().into());
        entry.insert("path".into(), call.path.clone().into());
        let storage_for = |header: &str| {
            options
                .storage
                .as_ref()
                .filter(|storage| storage.header.eq_ignore_ascii_case(header))
        };
        if let Some((header, _)) = &call.token
            && let Some(storage) = storage_for(header)
        {
            // The token the call sent as a header is one the site keeps in
            // its own storage, found in the sign-in window: a session itda
            // can read again at the next sign-in, not a static key the person
            // pastes. It is a `header` credential still, but its value lives
            // in the keyring under `storage:<key>`.
            if !auth_names.contains_key("storage") {
                let mut strategy = Mapping::new();
                strategy.insert("name".into(), "session".into());
                strategy.insert("kind".into(), "header".into());
                strategy.insert("header".into(), storage.header.clone().into());
                if !storage.prefix.is_empty() {
                    strategy.insert("prefix".into(), storage.prefix.clone().into());
                }
                strategy.insert(
                    "value_from".into(),
                    format!("storage:{}", storage.key).into(),
                );
                auth.push(Yaml::Mapping(strategy));
                auth_names.insert("storage".into(), "session".into());
            }
            entry.insert("auth".into(), "session".into());
        } else if let Some((header, bearer)) = &call.token {
            let key = header.to_ascii_lowercase();
            let auth_name = auth_names.entry(key.clone()).or_insert_with(|| {
                let auth_name = if key == "authorization" {
                    "token".to_owned()
                } else {
                    slug(header)
                };
                let mut strategy = Mapping::new();
                strategy.insert("name".into(), auth_name.clone().into());
                strategy.insert("kind".into(), "header".into());
                strategy.insert("header".into(), header.clone().into());
                if *bearer {
                    strategy.insert("prefix".into(), "Bearer ".into());
                }
                strategy.insert(
                    "value_from".into(),
                    format!("store:{site}/{auth_name}").into(),
                );
                auth.push(Yaml::Mapping(strategy));
                auth_name
            });
            entry.insert("auth".into(), auth_name.clone().into());
        } else if call.query_session > 0
            && let Some(key) = &query_session
        {
            if !auth_names.contains_key("query") {
                let mut strategy = Mapping::new();
                strategy.insert("name".into(), "session".into());
                strategy.insert("kind".into(), "browser".into());
                strategy.insert("value_from".into(), format!("query:{key}").into());
                if let Some(login) = &login {
                    strategy.insert("login".into(), Yaml::Mapping(login.clone()));
                }
                auth.push(Yaml::Mapping(strategy));
                auth_names.insert("query".into(), "session".into());
                // One session entry per site: a cookie session on top of a
                // query one would only name the same sign-in twice.
                auth_names.insert("cookie".into(), "session".into());
            }
            entry.insert("auth".into(), "session".into());
        } else if call.session.as_ref().is_some_and(|names| !names.is_empty()) {
            if !auth_names.contains_key("cookie") {
                let mut strategy = Mapping::new();
                strategy.insert("name".into(), "session".into());
                strategy.insert("kind".into(), "browser".into());
                if let Some(cookie) = &sign_in_cookie {
                    strategy.insert("value_from".into(), format!("cookie:{cookie}").into());
                }
                auth.push(Yaml::Mapping(strategy));
                auth_names.insert("cookie".into(), "session".into());
            }
            entry.insert("auth".into(), "session".into());
        }
        let read = match &call.graphql {
            Some(graphql) => !graphql.mutation,
            None => matches!(call.method.as_str(), "GET" | "HEAD"),
        };
        entry.insert("effect".into(), if read { "read" } else { "write" }.into());
        if name.starts_with("post-") && call.graphql.is_none() {
            notes.push(format!(
                "{label}: drafted as a change. If it only looks things up, set effect: read."
            ));
        }
        entry.insert(
            "evidence".into(),
            format!(
                "HAR recording: {label}, seen {} time{}",
                call.samples,
                if call.samples == 1 { "" } else { "s" }
            )
            .into(),
        );
        if !call.params.is_empty() {
            let mut params = Mapping::new();
            for param in &call.params {
                let mut field = Mapping::new();
                field.insert("type".into(), "string".into());
                field.insert("required".into(), true.into());
                params.insert(param.clone().into(), Yaml::Mapping(field));
            }
            entry.insert("params".into(), Yaml::Mapping(params));
        }
        if !call.query.0.is_empty() {
            entry.insert("query".into(), Yaml::Mapping(call.query.yaml(call.samples)));
        }
        if read && let Some(pagination) = pagination_of(call) {
            entry.insert("pagination".into(), pagination);
        }
        if let Some(header) = &call.header_token {
            match meta_page(entries, call, &origin) {
                Some((page, meta)) => {
                    let mut extract = Mapping::new();
                    extract.insert(
                        "token".into(),
                        format!("css:meta[name=\"{meta}\"]@content").into(),
                    );
                    let mut step = Mapping::new();
                    step.insert("name".into(), "page".into());
                    step_path(&mut step, &page, &prefix, &origin);
                    step.insert("extract".into(), Yaml::Mapping(extract));
                    entry.insert(
                        "preflight".into(),
                        Yaml::Sequence(vec![Yaml::Mapping(step)]),
                    );
                    let mut headers = Mapping::new();
                    headers.insert(header.clone().into(), "{{step.page.token}}".into());
                    entry.insert("headers".into(), Yaml::Mapping(headers));
                    notes.push(format!(
                        "{label}: a preflight step reads {header} fresh from the {meta} tag on {} before each call.",
                        page.path()
                    ));
                }
                None => notes.push(format!(
                    "{label}: left out the {header} header because no earlier recorded page had a csrf meta tag; add a preflight step to read it."
                )),
            }
        }
        let mut body = Mapping::new();
        let mut left_out = call.left_out.clone();
        if let Some(graphql) = &call.graphql {
            // The document and its name are fixed; the variables are the
            // inputs, rendered into the same JSON the page sent.
            let mut json = Mapping::new();
            let mut fixed = |key: &str, value: &str| {
                let mut field = Mapping::new();
                field.insert("type".into(), "string".into());
                field.insert("default".into(), value.into());
                json.insert(key.into(), Yaml::Mapping(field));
            };
            fixed("query", &graphql.document);
            if !graphql.name.is_empty() {
                fixed("operationName", &graphql.name);
            }
            let mut template = serde_json::Map::new();
            template.insert("query".into(), "{{query}}".into());
            if !graphql.name.is_empty() {
                template.insert("operationName".into(), "{{operationName}}".into());
            }
            let mut variables = serde_json::Map::new();
            for (variable, field) in graphql.variables.yaml(call.samples) {
                let Some(variable) = variable.as_str() else {
                    continue;
                };
                variables.insert(variable.to_owned(), format!("{{{{{variable}}}}}").into());
                json.insert(variable.into(), field);
            }
            if !variables.is_empty() {
                template.insert("variables".into(), Value::Object(variables));
            }
            body.insert("json".into(), Yaml::Mapping(json));
            body.insert(
                "json_template".into(),
                serde_norway::to_value(Value::Object(template)).unwrap_or(Yaml::Null),
            );
            if !graphql.complex.is_empty() {
                notes.push(format!(
                    "{label} ({name}): left out the object or list variables {}; add them to json_template by hand.",
                    graphql.complex.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        } else if !call.json.0.is_empty() {
            body.insert("json".into(), Yaml::Mapping(call.json.yaml(call.samples)));
        } else {
            let mut form = call.form.yaml(call.samples);
            if let Some(page) = token_page(entries, call, &origin) {
                let mut extract = Mapping::new();
                for token in &call.form_tokens {
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
                    form.insert(token.clone().into(), Yaml::Mapping(field));
                    left_out.remove(token);
                }
                let mut step = Mapping::new();
                step.insert("name".into(), "form".into());
                step_path(&mut step, &page, &prefix, &origin);
                step.insert("extract".into(), Yaml::Mapping(extract));
                let mut steps = match entry.remove("preflight") {
                    Some(Yaml::Sequence(steps)) => steps,
                    _ => Vec::new(),
                };
                steps.push(Yaml::Mapping(step));
                entry.insert("preflight".into(), Yaml::Sequence(steps));
                notes.push(format!(
                    "{label}: a preflight step reads {} fresh from {} before each change{}.",
                    call.form_tokens
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                    page.path(),
                    if page.query().is_some() {
                        "; add the query that page needs to the step's path"
                    } else {
                        ""
                    }
                ));
            }
            if !form.is_empty() {
                body.insert("form".into(), Yaml::Mapping(form));
            }
        }
        if call.multipart {
            let mut multipart = Mapping::new();
            let fields = call.parts.yaml(call.samples);
            if !fields.is_empty() {
                multipart.insert("fields".into(), Yaml::Mapping(fields));
            }
            if let Some(part) = &call.file_part {
                let mut from = Some(slug(part))
                    .filter(|from| !from.is_empty() && !secret_field(from))
                    .unwrap_or_else(|| "file".into());
                if call.parts.0.contains_key(&from) {
                    from = format!("{from}-file");
                }
                let mut file = Mapping::new();
                file.insert("name".into(), part.clone().into());
                file.insert("from".into(), from.clone().into());
                multipart.insert("file".into(), Yaml::Mapping(file));
                notes.push(format!(
                    "{label}: sends a file. The agent gives its path in {from}; you approve the file by name, size and hash."
                ));
            }
            body.insert("multipart".into(), Yaml::Mapping(multipart));
        }
        if !body.is_empty() {
            entry.insert("body".into(), Yaml::Mapping(body));
        }
        if read && !call.attachments.is_empty() {
            let mut response = Mapping::new();
            response.insert("format".into(), "file".into());
            response.insert(
                "media_types".into(),
                Yaml::Sequence(call.attachments.iter().cloned().map(Yaml::from).collect()),
            );
            entry.insert("response".into(), Yaml::Mapping(response));
            notes.push(format!(
                "{label}: the site sent it as an attachment; the read saves it as a file."
            ));
        } else if call.response_json {
            let mut response = Mapping::new();
            response.insert("format".into(), "json".into());
            let pointers: Vec<Yaml> = call
                .response_keys
                .iter()
                .flatten()
                .take(3)
                .map(|key| {
                    if key.starts_with('/') {
                        key.clone().into()
                    } else {
                        format!("/{}", key.replace('~', "~0").replace('/', "~1")).into()
                    }
                })
                .collect();
            if !pointers.is_empty() {
                response.insert("required_pointers".into(), Yaml::Sequence(pointers));
            }
            entry.insert("response".into(), Yaml::Mapping(response));
        }
        if !left_out.is_empty() {
            notes.push(format!(
                "{label}: left out {}; tokens like these come from sign-in or a preflight step, never from the agent.",
                left_out.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        operations.push(reorder(entry));
    }
    let host = base.host_str().unwrap_or_default().to_owned();
    Ok(Parts {
        site,
        title: String::new(),
        base_url,
        auth,
        operations,
        models: Mapping::new(),
        notes,
        source: format!("`itda draft har` from a recording of {host}"),
    })
}

/// The query key a site carries its session in, when the recording shows
/// one. Only a sign-in says which key that is: a `POST` whose path or body
/// is a sign-in, answered with a redirect whose address carries a key with a
/// session-looking value; or, failing that, a key with such a value on every
/// same-origin call after the sign-in and on none before it. A recording
/// without a sign-in names none.
pub fn query_session_key(entries: &[Value]) -> Option<String> {
    let sign_in = entries.iter().position(|entry| {
        sign_in_call(entry)
            && entry["request"]["method"]
                .as_str()
                .is_some_and(|m| m.eq_ignore_ascii_case("POST"))
    })?;
    let base = url::Url::parse(entries[sign_in]["request"]["url"].as_str()?).ok()?;
    // The redacted copy a refresh keeps blanks the value to `REDACTED`; it
    // still marks where the session was, so the copy drafts the same way.
    let looks = |value: &str| itda_types::url::session_value(value) || value == REDACTED;
    let location = entries[sign_in]["response"]["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("location"))
        })
        .and_then(|h| h["value"].as_str())
        .and_then(|to| base.join(to).ok());
    if let Some(target) = location
        && let Some((key, _)) = target.query_pairs().find(|(_, value)| looks(value))
    {
        return Some(key.into_owned());
    }
    let keys_of = |entry: &Value| -> Option<BTreeSet<String>> {
        let url = url::Url::parse(entry["request"]["url"].as_str()?).ok()?;
        if url.origin() != base.origin() {
            return None;
        }
        let mime = entry["response"]["content"]["mimeType"]
            .as_str()
            .unwrap_or_default();
        if ["image/", "font/", "javascript", "/css"]
            .iter()
            .any(|kind| mime.contains(kind))
        {
            return None;
        }
        Some(
            url.query_pairs()
                .filter(|(_, value)| looks(value))
                .map(|(key, _)| key.into_owned())
                .collect(),
        )
    };
    let after: Vec<BTreeSet<String>> = entries[sign_in + 1..].iter().filter_map(keys_of).collect();
    if after.len() < 2 {
        return None;
    }
    let before: BTreeSet<String> = entries[..sign_in]
        .iter()
        .filter_map(keys_of)
        .flatten()
        .collect();
    after
        .iter()
        .skip(1)
        .fold(after[0].clone(), |common, keys| {
            common.intersection(keys).cloned().collect()
        })
        .into_iter()
        .find(|key| !before.contains(key))
}

/// The query session a recording shows, as a [`crate::SessionAuth`] for the
/// page merge, so the pages read signed in carry `auth: session` too: the
/// query key, and the sign-in form when one was recorded. `None` when the
/// recording shows no query session.
pub(crate) fn query_session_auth(
    entries: &[Value],
    origin: &url::Origin,
    prefix: &str,
) -> Option<crate::SessionAuth> {
    let key = query_session_key(entries)?;
    let login = login_block(entries, origin, prefix)
        .map(|block| itda_spec::LoginForm {
            path: block
                .get("path")
                .and_then(Yaml::as_str)
                .unwrap_or_default()
                .to_owned(),
            user: block
                .get("user")
                .and_then(Yaml::as_str)
                .unwrap_or_default()
                .to_owned(),
            password: block
                .get("password")
                .and_then(Yaml::as_str)
                .unwrap_or_default()
                .to_owned(),
        })
        .unwrap_or_default();
    Some(crate::SessionAuth {
        cookie: String::new(),
        query: Some(key),
        login,
    })
}

/// The sign-in form as a definition's `login` block, from the last page
/// recorded before the sign-in that has one: where it is, and what its
/// user and password fields are called.
fn login_block(entries: &[Value], origin: &url::Origin, prefix: &str) -> Option<Mapping> {
    let sign_in = entries.iter().position(|entry| {
        sign_in_call(entry)
            && entry["request"]["method"]
                .as_str()
                .is_some_and(|m| m.eq_ignore_ascii_case("POST"))
    })?;
    entries[..sign_in].iter().rev().find_map(|entry| {
        if !html_page(entry) {
            return None;
        }
        let url = url::Url::parse(entry["request"]["url"].as_str()?).ok()?;
        if url.origin() != *origin {
            return None;
        }
        let html = entry["response"]["content"]["text"].as_str()?;
        let form = itda_html::evidence::sign_in_form(html, url.as_str(), None, None)?;
        let page = itda_types::url::sign_in_page(&url);
        let path = page.path().strip_prefix(prefix)?;
        let path = if path.is_empty() { "/" } else { path };
        let mut block = Mapping::new();
        block.insert(
            "path".into(),
            match page.query() {
                Some(query) => format!("{path}?{query}"),
                None => path.to_owned(),
            }
            .into(),
        );
        block.insert("user".into(), form.user.into());
        block.insert("password".into(), form.password.into());
        Some(block)
    })
}

/// What goes in the place of a recorded value that is kept on disk.
pub const REDACTED: &str = "<redacted>";

/// A recording with its secrets blanked, for the copy `itda refresh` keeps
/// under the site. The drafter reads names and shapes, never values, so the
/// copy keeps every name and blanks: every cookie's value in `Cookie` and
/// `Set-Cookie` (the names stay, so a sign-in cookie is still found), the
/// `Authorization` value after its scheme (`Bearer <redacted>`, so the
/// prefix is still drafted), every header whose name holds `token`, `key`,
/// `secret` or `session` or looks like a credential, query parameters and
/// form or JSON body fields with such names, and the whole response body of
/// a call that was a sign-in (a path segment such as `login`, or a body with
/// a password). A session the site carries in the query (the key a sign-in
/// handed out, see [`query_session_key`]) is blanked wherever it appears:
/// addresses, `Location` headers and the links in recorded pages.
pub fn redacted_recording(text: &[u8]) -> AppResult<Vec<u8>> {
    let mut doc: Value = serde_json::from_slice(text).map_err(|_| not_har())?;
    let Some(entries) = doc["log"]["entries"].as_array_mut() else {
        return Err(not_har());
    };
    let session_key = query_session_key(entries);
    let private = |name: &str| private_name(name) || session_key.as_deref() == Some(name);
    let blank_key = |text: &str| match &session_key {
        Some(key) => blank_query_key(text, key),
        None => text.to_owned(),
    };
    for entry in entries {
        let sign_in = sign_in_call(entry);
        if let Some(request) = entry.get_mut("request") {
            if let Some(url) = request
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_owned)
            {
                request["url"] = Value::String(redacted_url(&url, &private));
            }
            redact_pairs(request.get_mut("headers"), redact_header);
            redact_pairs(request.get_mut("cookies"), |_, _| Some(REDACTED.into()));
            redact_pairs(request.get_mut("queryString"), |name, _| {
                private(name).then(|| REDACTED.to_owned())
            });
            if let Some(post) = request.get_mut("postData") {
                redact_pairs(post.get_mut("params"), redact_named);
                let mime = post
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if let Some(body) = post.get("text").and_then(Value::as_str).map(str::to_owned) {
                    post["text"] = Value::String(redacted_body(&mime, &body));
                }
            }
        }
        if let Some(response) = entry.get_mut("response") {
            redact_pairs(response.get_mut("headers"), |name, value| {
                if name.eq_ignore_ascii_case("location") {
                    // A `Location` may be relative, so it is blanked by key
                    // rather than parsed as a whole address.
                    let blanked = blank_key(value);
                    return (blanked != value).then_some(blanked);
                }
                redact_header(name, value)
            });
            redact_pairs(response.get_mut("cookies"), |_, _| Some(REDACTED.into()));
            if let Some(content) = response.get_mut("content").filter(|c| c.is_object()) {
                if sign_in {
                    content["text"] = Value::String(String::new());
                    content["size"] = 0.into();
                } else if session_key.is_some()
                    && let Some(text) = content.get("text").and_then(Value::as_str)
                {
                    let blanked = blank_key(text);
                    if blanked != text {
                        content["text"] = Value::String(blanked);
                    }
                }
            }
        }
    }
    Ok(serde_json::to_vec(&doc)?)
}

/// `text` with the value after every `key=` blanked, where `key` starts a
/// query parameter (after `?`, `&` or an HTML `&amp;`). Links in a recorded
/// page carry a query session the way addresses do.
fn blank_query_key(text: &str, key: &str) -> String {
    let marker = format!("{key}=");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&marker) {
        let (head, tail) = rest.split_at(at);
        let starts_parameter = head
            .chars()
            .next_back()
            .is_some_and(|c| matches!(c, '?' | '&' | ';'));
        out.push_str(head);
        let after = &tail[marker.len()..];
        if !starts_parameter {
            out.push_str(&marker);
            rest = after;
            continue;
        }
        let end = after
            .find(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '%'))
            })
            .unwrap_or(after.len());
        out.push_str(&marker);
        if end > 0 {
            out.push_str(REDACTED);
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// A name that holds a credential: the words `token`, `key`, `secret` or
/// `session`, or what [`secret_field`] knows (`password`, `csrf`...).
fn private_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["token", "key", "secret", "session", "sess"]
        .iter()
        .any(|word| lower.contains(word))
        || lower == "sid"
        || secret_field(name)
}

/// A call that signed the person in: its path says so, or its body carries
/// a password.
fn sign_in_call(entry: &Value) -> bool {
    let request = &entry["request"];
    let by_path = request["url"]
        .as_str()
        .and_then(|url| url::Url::parse(url).ok())
        .is_some_and(|url| {
            url.path().split('/').any(|segment| {
                matches!(
                    segment.to_ascii_lowercase().as_str(),
                    "login" | "signin" | "sign-in" | "sign_in" | "sso" | "oauth" | "authenticate"
                )
            })
        });
    let password = |name: &str| {
        let lower = name.to_ascii_lowercase();
        lower.contains("password") || lower.contains("passwd")
    };
    let in_params = request["postData"]["params"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|p| p["name"].as_str().is_some_and(password));
    let in_text = request["postData"]["text"].as_str().is_some_and(|text| {
        let mime = request["postData"]["mimeType"].as_str().unwrap_or_default();
        if mime.contains("json") {
            serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| v.as_object().map(|o| o.keys().any(|k| password(k))))
                .unwrap_or(false)
        } else {
            url::form_urlencoded::parse(text.as_bytes()).any(|(name, _)| password(&name))
        }
    });
    by_path || in_params || in_text
}

/// Blank the `value` of every `{name, value}` pair `redact` says to.
fn redact_pairs(pairs: Option<&mut Value>, redact: impl Fn(&str, &str) -> Option<String>) {
    for pair in pairs.and_then(Value::as_array_mut).into_iter().flatten() {
        let name = pair["name"].as_str().unwrap_or_default().to_owned();
        let value = pair["value"].as_str().unwrap_or_default().to_owned();
        if let Some(blanked) = redact(&name, &value) {
            pair["value"] = Value::String(blanked);
        }
    }
}

fn redact_named(name: &str, _value: &str) -> Option<String> {
    private_name(name).then(|| REDACTED.to_owned())
}

/// A header's value with its secret blanked: cookies keep their names, the
/// `Authorization` header keeps its scheme.
fn redact_header(name: &str, value: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "cookie" => Some(
            value
                .split(';')
                .map(|pair| match pair.split_once('=') {
                    Some((name, _)) => format!("{}={REDACTED}", name.trim()),
                    None => pair.trim().to_owned(),
                })
                .collect::<Vec<_>>()
                .join("; "),
        ),
        "set-cookie" => Some(match value.split_once('=') {
            Some((name, _)) => format!("{}={REDACTED}", name.trim()),
            None => REDACTED.to_owned(),
        }),
        "authorization" | "proxy-authorization" => Some(match value.split_once(' ') {
            Some((scheme, _)) if !scheme.is_empty() => format!("{scheme} {REDACTED}"),
            _ => REDACTED.to_owned(),
        }),
        _ => private_name(name).then(|| REDACTED.to_owned()),
    }
}

/// The address with the values of private query parameters blanked.
fn redacted_url(url: &str, private: &dyn Fn(&str) -> bool) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.to_owned();
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(name, value)| {
            let value = if private(&name) {
                REDACTED.to_owned()
            } else {
                value.into_owned()
            };
            (name.into_owned(), value)
        })
        .collect();
    if pairs.iter().all(|(name, _)| !private(name)) {
        return url.to_owned();
    }
    parsed.query_pairs_mut().clear().extend_pairs(pairs);
    parsed.into()
}

/// A request body with its private fields blanked: JSON fields by name at
/// any depth, form fields by name. Other bodies are left as they are.
fn redacted_body(mime: &str, text: &str) -> String {
    if mime.contains("json") {
        let Ok(mut value) = serde_json::from_str::<Value>(text) else {
            return text.to_owned();
        };
        redact_json(&mut value);
        return value.to_string();
    }
    if mime.contains("x-www-form-urlencoded") {
        let mut out = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in url::form_urlencoded::parse(text.as_bytes()) {
            if private_name(&name) {
                out.append_pair(&name, REDACTED);
            } else {
                out.append_pair(&name, &value);
            }
        }
        return out.finish();
    }
    text.to_owned()
}

fn redact_json(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (name, child) in map.iter_mut() {
                if private_name(name) && !child.is_null() {
                    *child = Value::String(REDACTED.into());
                } else {
                    redact_json(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        _ => {}
    }
}

/// A page the person saw: a successful `GET` whose answer was HTML with
/// its text recorded.
/// The media type of an answer the site marked `Content-Disposition:
/// attachment`, for a successful GET; `None` for every other answer.
fn attachment_answer(entry: &Value) -> Option<String> {
    if entry["request"]["method"].as_str() != Some("GET")
        || !entry["response"]["status"]
            .as_u64()
            .is_some_and(|s| (200..300).contains(&s))
    {
        return None;
    }
    let attachment = entry["response"]["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|header| {
            header["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("content-disposition"))
                && header["value"]
                    .as_str()
                    .and_then(|v| v.split(';').next())
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("attachment"))
        });
    if !attachment {
        return None;
    }
    let media = entry["response"]["content"]["mimeType"]
        .as_str()
        .and_then(|m| m.split(';').next())
        .map(|m| m.trim().to_ascii_lowercase())
        .filter(|m| m.contains('/'))
        .unwrap_or_else(|| "application/octet-stream".into());
    Some(media)
}

pub(crate) fn html_page(entry: &Value) -> bool {
    let content = &entry["response"]["content"];
    entry["request"]["method"].as_str() == Some("GET")
        && entry["response"]["status"]
            .as_u64()
            .is_some_and(|s| (200..300).contains(&s))
        && content["mimeType"]
            .as_str()
            .is_some_and(|m| m.contains("html"))
        && content["encoding"].as_str() != Some("base64")
        && content["text"].as_str().is_some_and(|t| !t.is_empty())
}

fn not_har() -> itda_types::error::AppError {
    usage(
        "This is not a HAR recording",
        "In the browser's network panel, choose Save all as HAR (with content).",
    )
}

/// Only the path: query values, like every recorded value, stay out.
fn step_path(step: &mut Mapping, page: &url::Url, prefix: &str, origin: &url::Origin) {
    match page.path().strip_prefix(prefix) {
        Some(rest) => step.insert("path".into(), rest.into()),
        None => {
            step.insert("base_url".into(), origin.ascii_serialization().into());
            step.insert("path".into(), page.path().into())
        }
    };
}

/// The latest HTML page on the same origin, recorded before the call, whose
/// text `wanted` picks something from.
fn page_before<T>(
    entries: &[Value],
    call: &Call,
    origin: &url::Origin,
    wanted: impl Fn(&str) -> Option<T>,
) -> Option<(url::Url, T)> {
    entries[..call.first?].iter().rev().find_map(|entry| {
        let url = url::Url::parse(entry["request"]["url"].as_str()?).ok()?;
        let content = &entry["response"]["content"];
        let html = entry["request"]["method"].as_str() == Some("GET")
            && entry["response"]["status"]
                .as_u64()
                .is_some_and(|s| (200..300).contains(&s))
            && content["mimeType"]
                .as_str()
                .is_some_and(|m| m.contains("html"));
        if !html || url.origin() != *origin {
            return None;
        }
        Some((url, wanted(content["text"].as_str()?)?))
    })
}

/// The page whose form holds every token field the call sent: the latest
/// one, as the form the person filled in.
fn token_page(entries: &[Value], call: &Call, origin: &url::Origin) -> Option<url::Url> {
    if call.form_tokens.is_empty() {
        return None;
    }
    let found = page_before(entries, call, origin, |text| {
        let has = |token: &str| {
            ["\"", "'", ""]
                .iter()
                .any(|quote| text.contains(&format!("name={quote}{token}{quote}")))
        };
        call.form_tokens.iter().all(|t| has(t)).then_some(())
    });
    found.map(|(url, ())| url)
}

/// The page with a meta tag such as `<meta name="csrf-token" content="…">`,
/// which Rails, Laravel and many single-page apps send back as a header.
fn meta_page(entries: &[Value], call: &Call, origin: &url::Origin) -> Option<(url::Url, String)> {
    page_before(entries, call, origin, csrf_meta)
}

fn csrf_meta(text: &str) -> Option<String> {
    let names: Vec<String> = text
        .split("<meta")
        .skip(1)
        .filter_map(|tag| {
            let tag = &tag[..tag.find('>')?];
            let rest = &tag[tag.find("name=")? + 5..];
            let name = match rest.chars().next()? {
                quote @ ('"' | '\'') => rest[1..].split(quote).next()?,
                _ => rest.split([' ', '/']).next()?,
            };
            let lower = name.to_ascii_lowercase();
            let safe = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            (safe && (lower.contains("csrf") || lower.contains("xsrf"))).then(|| name.to_owned())
        })
        .collect();
    // Rails pages carry both `csrf-param` (a field name) and `csrf-token`.
    names
        .iter()
        .find(|name| name.to_ascii_lowercase().contains("token"))
        .or_else(|| names.first())
        .cloned()
}

/// Paths pages post their own usage to; nothing an agent should repeat.
fn telemetry(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.split(['/', '.', '-', '_']).any(|part| {
        matches!(
            part,
            "envelope"
                | "collect"
                | "beacon"
                | "analytics"
                | "telemetry"
                | "track"
                | "tracking"
                | "metrics"
                | "sentry"
                | "rum"
                | "pixel"
                | "ping"
        )
    })
}

fn api_call(entry: &Value) -> bool {
    let status = entry["response"]["status"].as_u64().unwrap_or(0);
    let json = entry["response"]["content"]["mimeType"]
        .as_str()
        .is_some_and(|m| m.contains("json"));
    let method = entry["request"]["method"].as_str().unwrap_or("GET");
    match status {
        200..=299 => {
            json || !matches!(method, "GET" | "HEAD" | "OPTIONS")
                || attachment_answer(entry).is_some()
        }
        300..=399 => !matches!(method, "GET" | "HEAD" | "OPTIONS"),
        _ => false,
    }
}

fn busiest_origin(recorded: &[(&Value, url::Url)]) -> Option<url::Url> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, url) in recorded {
        *counts
            .entry(url.origin().ascii_serialization())
            .or_default() += 1;
    }
    let (origin, _) = counts.into_iter().max_by_key(|(_, n)| *n)?;
    url::Url::parse(&origin).ok()
}

fn record_body(call: &mut Call, post: &Value) {
    let mime = post["mimeType"].as_str().unwrap_or_default();
    let text = post["text"].as_str().unwrap_or_default();
    if mime.contains("multipart") {
        call.multipart = true;
        // A recording keeps the parts' names, and a file part's file name,
        // never the file; a part without a name says nothing.
        for part in post["params"].as_array().into_iter().flatten() {
            let Some(name) = part["name"].as_str().filter(|n| !n.is_empty()) else {
                continue;
            };
            if part["fileName"].is_string() {
                call.file_part.get_or_insert_with(|| name.to_owned());
            } else {
                if secret_field(name) {
                    call.form_tokens.insert(name.to_owned());
                }
                add_input(
                    &mut call.parts,
                    &mut call.left_out,
                    name,
                    Kind::of_text(part["value"].as_str().unwrap_or_default()),
                );
            }
        }
    } else if mime.contains("json") {
        if let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(text) {
            if let Some(graphql) = &mut call.graphql {
                for (name, value) in fields["variables"].as_object().into_iter().flatten() {
                    match value {
                        Value::Null => {}
                        Value::Object(_) | Value::Array(_) => {
                            graphql.complex.insert(name.clone());
                        }
                        _ => add_input(
                            &mut graphql.variables,
                            &mut call.left_out,
                            name,
                            Kind::of(value),
                        ),
                    }
                }
                return;
            }
            for (name, value) in &fields {
                add_input(&mut call.json, &mut call.left_out, name, Kind::of(value));
            }
        }
    } else if mime.contains("x-www-form-urlencoded") {
        for (name, value) in url::form_urlencoded::parse(text.as_bytes()) {
            if secret_field(&name) {
                call.form_tokens.insert(name.clone().into_owned());
            }
            add_input(
                &mut call.form,
                &mut call.left_out,
                &name,
                Kind::of_text(&value),
            );
        }
    }
}

fn add_input(fields: &mut Fields, left_out: &mut BTreeSet<String>, name: &str, kind: Kind) {
    if secret_field(name) {
        left_out.insert(name.to_owned());
    } else {
        fields.add(name, kind);
    }
}

fn record_response(call: &mut Call, content: &Value) {
    if !content["mimeType"]
        .as_str()
        .is_some_and(|m| m.contains("json"))
    {
        return;
    }
    call.response_json = true;
    if content["encoding"].as_str() == Some("base64") {
        return;
    }
    let keys: BTreeSet<String> = match content["text"].as_str().map(serde_json::from_str::<Value>) {
        // A GraphQL answer is checked inside `data`, where the fields are.
        Some(Ok(Value::Object(map))) if call.graphql.is_some() => map
            .get("data")
            .and_then(Value::as_object)
            .map(|data| {
                data.keys()
                    .map(|k| format!("/data/{}", k.replace('~', "~0").replace('/', "~1")))
                    .collect()
            })
            .unwrap_or_default(),
        Some(Ok(Value::Object(map))) => map.keys().cloned().collect(),
        _ => BTreeSet::new(),
    };
    call.response_keys = Some(match call.response_keys.take() {
        Some(seen) => seen.intersection(&keys).cloned().collect(),
        None => keys,
    });
    if call.graphql.is_some() {
        let found = content["text"]
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|answer| {
                answer["data"].as_object().and_then(|data| {
                    data.iter().find_map(|(key, value)| {
                        let info = &value["pageInfo"];
                        info["endCursor"]
                            .as_str()
                            .map(|_| (key.clone(), info["hasNextPage"].is_boolean()))
                    })
                })
            });
        call.page_info = match (call.page_info.take(), found, call.samples) {
            (None, found, 1) => found,
            (Some(seen), Some(found), _) if seen == found => Some(seen),
            _ => None,
        };
    }
}

/// How a recorded read turned pages, when its query or variables and its
/// answers say so plainly: a cursor the answer handed back, a page number
/// or an offset.
fn pagination_of(call: &Call) -> Option<Yaml> {
    if let Some(graphql) = &call.graphql {
        let (field, has_next) = call.page_info.as_ref()?;
        let after = graphql
            .variables
            .0
            .iter()
            .find(|(name, (_, kind))| name.as_str() == "after" && *kind == Kind::Str)?;
        let more = if *has_next {
            format!("/data/{field}/pageInfo/hasNextPage")
        } else {
            String::new()
        };
        return Some(crate::pagination_yaml(
            "cursor",
            &format!("/data/{field}/pageInfo/endCursor"),
            &more,
            after.0,
            true,
        ));
    }
    let strings = call
        .query
        .0
        .iter()
        .filter(|(_, (_, kind))| *kind == Kind::Str)
        .map(|(name, _)| name.as_str());
    if let Some(input) = crate::cursor_input(strings)
        && let Some(next) = crate::next_cursor_key(
            call.response_keys
                .iter()
                .flatten()
                .map(String::as_str)
                .filter(|key| !key.starts_with('/')),
        )
    {
        return Some(crate::pagination_yaml(
            "cursor",
            &format!("/{}", next.replace('~', "~0").replace('/', "~1")),
            "",
            input,
            false,
        ));
    }
    let ints = call
        .query
        .0
        .iter()
        .filter(|(_, (_, kind))| *kind == Kind::Int)
        .map(|(name, _)| name.as_str());
    let (input, style) = crate::counted_input(ints)?;
    Some(crate::pagination_yaml(style, "", "", input, false))
}

/// Turns ids in a path into `{name}` placeholders named after the segment
/// before them: `/notices/42` becomes `/notices/{notice_id}`.
pub(crate) fn template(path: &str) -> (String, Vec<String>) {
    let mut out = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut previous = "";
    for segment in path.split('/') {
        if looks_like_id(segment) {
            let stem = slug(&singular(previous)).replace('-', "_");
            let base = if stem.is_empty() {
                "id".to_owned()
            } else {
                format!("{stem}_id")
            };
            let mut name = base.clone();
            let mut n = 2;
            while params.contains(&name) {
                name = format!("{base}{n}");
                n += 1;
            }
            out.push(format!("{{{name}}}"));
            params.push(name);
            previous = "";
        } else {
            out.push(segment.to_owned());
            previous = segment;
        }
    }
    let path = out.join("/");
    (
        if path.starts_with('/') {
            path
        } else {
            format!("/{path}")
        },
        params,
    )
}

fn looks_like_id(segment: &str) -> bool {
    let hex = segment.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    !segment.is_empty()
        && (segment.chars().all(|c| c.is_ascii_digit())
            || (hex && segment.len() >= 16 && segment.chars().any(|c| c.is_ascii_digit()))
            || (segment.len() >= 24
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && segment.chars().any(|c| c.is_ascii_digit())))
}

/// `article` is `articles`, `entry` is `entries`; a word already plural, or
/// one with no plural, stays.
pub(crate) fn plural(word: &str) -> String {
    if word.is_empty()
        || word.ends_with('s')
        || matches!(word, "content" | "data" | "news" | "media" | "people")
    {
        return word.to_owned();
    }
    if let Some(stem) = word.strip_suffix('y')
        && !stem.ends_with(['a', 'e', 'i', 'o', 'u'])
    {
        return format!("{stem}ies");
    }
    if word.ends_with("ch") || word.ends_with("sh") || word.ends_with('x') {
        return format!("{word}es");
    }
    format!("{word}s")
}

pub(crate) fn singular(word: &str) -> String {
    if let Some(stem) = word.strip_suffix("ies") {
        format!("{stem}y")
    } else if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") {
        word[..word.len() - 1].to_owned()
    } else {
        word.to_owned()
    }
}

/// `GET /api/v1/notices` is `list-notices`, `GET /notices/{id}` is
/// `get-notice`, `POST /notices` is `create-notice`.
pub(crate) fn action_name(method: &str, path: &str) -> String {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let ends_with_param = segments.last().is_some_and(|s| s.starts_with('{'));
    let resource = segments
        .iter()
        .rev()
        .find(|s| !s.starts_with('{') && !is_prefix(s))
        .map(|s| s.split('.').next().unwrap_or(s))
        .unwrap_or("root");
    let one = singular(resource);
    let name = match method {
        "GET" | "HEAD" if !ends_with_param && one != resource => format!("list-{resource}"),
        "GET" | "HEAD" => format!("get-{one}"),
        "POST" if !ends_with_param && one != resource => format!("create-{one}"),
        "POST" => format!("post-{resource}"),
        "PUT" | "PATCH" => format!("update-{one}"),
        "DELETE" => format!("delete-{one}"),
        other => format!("{}-{resource}", other.to_lowercase()),
    };
    slug(&name)
}

fn is_prefix(segment: &str) -> bool {
    matches!(segment, "api" | "rest" | "ajax")
        || (segment.len() > 1
            && segment.starts_with('v')
            && segment[1..].chars().all(|c| c.is_ascii_digit()))
}

pub(crate) fn unique(names: &mut BTreeSet<String>, name: &str) -> String {
    let name = if name.is_empty() { "action" } else { name };
    let mut candidate = name.to_owned();
    let mut n = 2;
    while !names.insert(candidate.clone()) {
        candidate = format!("{name}-{n}");
        n += 1;
    }
    candidate
}

/// `intranet.example.co.kr` becomes `intranet-example`.
pub(crate) fn site_name(host: &str) -> String {
    let labels: Vec<&str> = host
        .split('.')
        .filter(|l| !matches!(*l, "www" | "api" | "co" | "or" | "go" | "ac" | "ne"))
        .collect();
    let kept = if labels.len() > 1 {
        &labels[..labels.len() - 1]
    } else {
        &labels[..]
    };
    let name = slug(&kept.join("-"));
    if name.is_empty() { "site".into() } else { name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_become_templates() {
        assert_eq!(
            template("/api/notices/42/comments/9"),
            (
                "/api/notices/{notice_id}/comments/{comment_id}".into(),
                vec!["notice_id".into(), "comment_id".into()]
            )
        );
        assert_eq!(
            template("/files/3f2a9c1e-7b4d-4e1a-9c2b-5d6e7f8a9b0c").0,
            "/files/{file_id}"
        );
        assert_eq!(template("/v2/status").0, "/v2/status");
        assert_eq!(template("/42").0, "/{id}");
    }

    #[test]
    fn names_read_like_actions() {
        assert_eq!(action_name("GET", "/api/v1/notices"), "list-notices");
        assert_eq!(action_name("GET", "/api/notices/{notice_id}"), "get-notice");
        assert_eq!(action_name("POST", "/api/notices"), "create-notice");
        assert_eq!(action_name("POST", "/board/write.do"), "post-write");
        assert_eq!(
            action_name("DELETE", "/api/notices/{notice_id}"),
            "delete-notice"
        );
        assert_eq!(action_name("GET", "/api/me"), "get-me");
        assert_eq!(site_name("intranet.example.co.kr"), "intranet-example");
        assert_eq!(site_name("www.example.com"), "example");
    }
}
