//! Browser recordings (HAR files) to a draft definition.
//!
//! A recording holds real sign-in headers, cookies and personal data, so only
//! the shape of each call is kept: method, path, input names and types, and the
//! response fields every sample had. No header, cookie, query or body value is
//! copied into the draft.
use crate::{Draft, Options, reorder, slug};
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
enum Kind {
    Int,
    Float,
    Bool,
    Str,
}

impl Kind {
    fn of(value: &Value) -> Kind {
        match value {
            Value::Bool(_) => Kind::Bool,
            Value::Number(n) if n.is_i64() || n.is_u64() => Kind::Int,
            Value::Number(_) => Kind::Float,
            Value::String(s) => Kind::of_text(s),
            _ => Kind::Str,
        }
    }

    fn of_text(text: &str) -> Kind {
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

    fn name(self) -> &'static str {
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
    left_out: BTreeSet<String>,
    /// Top-level response fields present in every JSON sample.
    response_keys: Option<BTreeSet<String>>,
    response_json: bool,
    token: Option<(String, bool)>,
}

pub fn from_har(text: &[u8], options: &Options) -> AppResult<Draft> {
    let doc: Value = serde_json::from_slice(text).map_err(|_| not_har())?;
    let entries = doc["log"]["entries"].as_array().ok_or_else(not_har)?;
    let recorded: Vec<(&Value, url::Url)> = entries
        .iter()
        .filter(|entry| api_call(entry))
        .filter_map(|entry| {
            Some((
                entry,
                url::Url::parse(entry["request"]["url"].as_str()?).ok()?,
            ))
        })
        .filter(|(_, url)| matches!(url.scheme(), "http" | "https"))
        .collect();

    let base = match &options.base_url {
        Some(base) => url::Url::parse(base.trim_end_matches('/'))
            .map_err(|_| usage("--base-url is not a URL", "Pass an address like https://example.com/api."))?,
        None => busiest_origin(&recorded).ok_or_else(|| {
            usage(
                "The recording has no API calls",
                "Record again with the browser's network panel open while you use the site, then save it as HAR with content.",
            )
        })?,
    };
    let origin = base.origin();
    let prefix = base.path().trim_end_matches('/').to_owned();
    let base_url = format!("{}{prefix}", origin.ascii_serialization());

    let mut notes = Vec::new();
    let mut elsewhere = BTreeSet::new();
    let mut calls: BTreeMap<(String, String), Call> = BTreeMap::new();
    let mut cookies = false;
    for (entry, url) in &recorded {
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
        let call = calls.entry((path.clone(), method.clone())).or_default();
        call.method = method;
        call.path = path;
        call.params = params;
        call.samples += 1;
        for (name, value) in url.query_pairs() {
            if name == "_" {
                continue;
            }
            if secret_field(&name) {
                call.left_out.insert(name.into_owned());
            } else {
                call.query.add(&name, Kind::of_text(&value));
            }
        }
        record_body(call, &request["postData"]);
        for header in request["headers"].as_array().into_iter().flatten() {
            let name = header["name"].as_str().unwrap_or_default();
            let lower = name.to_ascii_lowercase();
            if lower == "cookie" {
                cookies = true;
            } else if TOKEN_HEADERS.contains(&lower.as_str()) {
                let bearer = header["value"]
                    .as_str()
                    .is_some_and(|v| v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer "));
                call.token = Some((name.to_owned(), bearer));
            }
        }
        record_response(call, &entry["response"]["content"]);
    }
    if !elsewhere.is_empty() {
        notes.push(format!(
            "Left out calls to other addresses: {}.",
            elsewhere.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if cookies {
        notes.push(
            "The site sends cookies; if it needs you signed in, these actions will report auth_required until browser sign-in is set up."
                .into(),
        );
    }

    let site = options
        .name
        .as_deref()
        .map(slug)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| site_name(base.host_str().unwrap_or_default()));
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
        if call.multipart {
            notes.push(format!(
                "{label}: skipped; it uploads files, which drafts do not cover yet."
            ));
            continue;
        }
        let name = unique(&mut names, &action_name(&call.method, &call.path));
        let mut entry = Mapping::new();
        entry.insert("name".into(), name.clone().into());
        entry.insert("method".into(), call.method.clone().into());
        entry.insert("path".into(), call.path.clone().into());
        if let Some((header, bearer)) = &call.token {
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
        }
        let read = matches!(call.method.as_str(), "GET" | "HEAD");
        entry.insert("effect".into(), if read { "read" } else { "write" }.into());
        if name.starts_with("post-") {
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
        let mut body = Mapping::new();
        if !call.json.0.is_empty() {
            body.insert("json".into(), Yaml::Mapping(call.json.yaml(call.samples)));
        } else if !call.form.0.is_empty() {
            body.insert("form".into(), Yaml::Mapping(call.form.yaml(call.samples)));
        }
        if !body.is_empty() {
            entry.insert("body".into(), Yaml::Mapping(body));
        }
        if call.response_json {
            let mut response = Mapping::new();
            response.insert("format".into(), "json".into());
            let pointers: Vec<Yaml> = call
                .response_keys
                .iter()
                .flatten()
                .take(3)
                .map(|key| format!("/{}", key.replace('~', "~0").replace('/', "~1")).into())
                .collect();
            if !pointers.is_empty() {
                response.insert("required_pointers".into(), Yaml::Sequence(pointers));
            }
            entry.insert("response".into(), Yaml::Mapping(response));
        }
        if !call.left_out.is_empty() {
            notes.push(format!(
                "{label}: left out {}; tokens like these come from sign-in or a preflight step, never from the agent.",
                call.left_out.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        operations.push(reorder(entry));
    }
    if operations.is_empty() {
        return Err(usage(
            "No operations could be drafted",
            "Check that the recording has API calls on this address, or loosen --include.",
        ));
    }
    let host = base.host_str().unwrap_or_default().to_owned();
    Ok(crate::finish(
        &site,
        "",
        &base_url,
        auth,
        operations,
        notes,
        &format!("`itda draft har` from a recording of {host}"),
    ))
}

fn not_har() -> itda_types::error::AppError {
    usage(
        "This is not a HAR recording",
        "In the browser's network panel, choose Save all as HAR (with content).",
    )
}

/// Calls a page makes to its API: JSON answers, and changes sent with a body.
fn api_call(entry: &Value) -> bool {
    let status = entry["response"]["status"].as_u64().unwrap_or(0);
    let json = entry["response"]["content"]["mimeType"]
        .as_str()
        .is_some_and(|m| m.contains("json"));
    let method = entry["request"]["method"].as_str().unwrap_or("GET");
    match status {
        200..=299 => json || !matches!(method, "GET" | "HEAD" | "OPTIONS"),
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
    } else if mime.contains("json") {
        if let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(text) {
            for (name, value) in &fields {
                add_input(&mut call.json, &mut call.left_out, name, Kind::of(value));
            }
        }
    } else if mime.contains("x-www-form-urlencoded") {
        for (name, value) in url::form_urlencoded::parse(text.as_bytes()) {
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
        Some(Ok(Value::Object(map))) => map.keys().cloned().collect(),
        _ => BTreeSet::new(),
    };
    call.response_keys = Some(match call.response_keys.take() {
        Some(seen) => seen.intersection(&keys).cloned().collect(),
        None => keys,
    });
}

/// Turns ids in a path into `{name}` placeholders named after the segment
/// before them: `/notices/42` becomes `/notices/{notice_id}`.
fn template(path: &str) -> (String, Vec<String>) {
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

fn singular(word: &str) -> String {
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
fn action_name(method: &str, path: &str) -> String {
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

fn unique(names: &mut BTreeSet<String>, name: &str) -> String {
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
fn site_name(host: &str) -> String {
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
