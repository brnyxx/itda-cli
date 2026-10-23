//! Holding an agent's draft to what itda saw on the site. An agent that
//! drafts a definition from evidence may still write an operation nothing
//! showed: a server the site never named, a route no page or description
//! has, an HTML read with nothing to check. Each operation is kept only
//! when the evidence carries it, and every judgment says which evidence it
//! rests on or what is missing. Pure: the caller collects what was seen.
use itda_policy::{Effect, operation_effect, rendered_request_effect};
use itda_spec::{Operation, Spec};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The evidence a drafted operation is judged with until propose names it.
const DRAFTED: &str = "itda_prepare";

/// A server the site documented (OpenAPI `servers`, an absolute address in
/// its docs) or answered from (a page itda read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenServer {
    pub url: String,
    pub evidence: String,
}

/// A route the docs, a description or a script named. `path` is the whole
/// path on its origin; `origin` is the origin it was written with, when it
/// was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenRoute {
    pub method: String,
    pub path: String,
    pub origin: Option<String>,
    pub evidence: String,
    /// The description says the route needs a sign-in itda cannot use.
    pub unsupported_auth: bool,
}

/// A path itda fetched, or saw a page of the site link to: the origin, the
/// path and the evidence. `session` says it was read with an account's
/// session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenPath {
    pub origin: String,
    pub path: String,
    pub evidence: String,
    pub session: bool,
}

/// Everything the judgments rest on.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    pub servers: Vec<SeenServer>,
    pub routes: Vec<SeenRoute>,
    pub paths: Vec<SeenPath>,
}

/// What became of one operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Judgment {
    pub op: String,
    /// `kept` or `dropped`.
    pub verdict: &'static str,
    /// `read` or `change`, as itda classifies it, not as it was declared.
    pub effect: &'static str,
    /// Why, in fixed words.
    pub reason: String,
    /// The evidence the verdict rests on.
    pub evidence_ids: Vec<String>,
    /// What evidence would have kept it, when it was dropped.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// For a kept change: a kept read that could confirm it landed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm_with: Option<String>,
}

/// The judgments of a draft, and the refusal of the whole draft when what
/// is left gives an agent nothing (`no_actions`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Grounded {
    pub judgments: Vec<Judgment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_actions: Option<String>,
}

impl Grounded {
    /// The names of the operations kept.
    pub fn kept(&self) -> Vec<&str> {
        if self.no_actions.is_some() {
            return Vec::new();
        }
        self.judgments
            .iter()
            .filter(|j| j.verdict == "kept")
            .map(|j| j.op.as_str())
            .collect()
    }
}

/// The extensions of a file a read would save rather than read.
const FILE_EXTENSIONS: [&str; 12] = [
    ".pdf", ".csv", ".xlsx", ".xls", ".zip", ".docx", ".doc", ".pptx", ".hwp", ".hwpx", ".gz",
    ".tar",
];

/// Path segments a site puts in front of its routes: `/api`, `/v10`,
/// `/rest`. A documented `/users/@me` is the same route as
/// `/api/v10/users/@me`.
fn prefix_like(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    matches!(lower.as_str(), "api" | "rest" | "public" | "json" | "ajax")
        || (lower.len() > 1
            && lower.starts_with('v')
            && lower[1..].chars().all(|c| c.is_ascii_digit() || c == '.'))
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn variable(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

/// Whether two route templates name the same segments; a `{variable}`
/// matches any one segment.
fn same_segments(a: &[&str], b: &[&str]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x == y || variable(x) || variable(y))
}

/// How loosely a route fits a path: the segments where only one of the two
/// is a `{variable}`. Of the routes that fit, the closest is cited.
fn looseness(seen: &str, wanted: &str) -> usize {
    let (a, b) = (segments(seen), segments(wanted));
    let n = a.len().min(b.len());
    a[a.len() - n..]
        .iter()
        .zip(&b[b.len() - n..])
        .filter(|(x, y)| variable(x) != variable(y))
        .count()
}

/// Whether the route `seen` names the path `wanted`, allowing either to
/// carry extra leading segments that only say "API" or a version.
pub fn route_matches(seen: &str, wanted: &str) -> bool {
    let (a, b) = (segments(seen), segments(wanted));
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let extra = long.len() - short.len();
    long[..extra].iter().all(|s| prefix_like(s)) && same_segments(&short, &long[extra..])
}

/// Whether a template segment fits a segment of a path itda saw: the
/// same words, or a `{variable}` with whatever stands there, as a whole
/// segment (`{id}` and `42`) or with the same words around it
/// (`{page}.html` and `re.html`).
fn segment_fits(template: &str, seen: &str) -> bool {
    if template == seen {
        return true;
    }
    let Some((before, rest)) = template.split_once('{') else {
        return false;
    };
    let Some((_, after)) = rest.split_once('}') else {
        return false;
    };
    seen.len() > before.len() + after.len() && seen.starts_with(before) && seen.ends_with(after)
}

/// Whether a path itda saw fits an operation's path template, segment by
/// segment.
fn path_fits(template: &str, seen: &str) -> bool {
    let (t, s) = (segments(template), segments(seen));
    t.len() == s.len() && t.iter().zip(&s).all(|(a, b)| segment_fits(a, b))
}

/// The origin and the path of an address, without a trailing `/`.
fn origin_and_path(url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url).ok()?;
    Some((
        parsed.origin().ascii_serialization(),
        parsed.path().trim_end_matches('/').to_owned(),
    ))
}

/// The whole path an operation is sent to on its origin.
fn full_path(base_path: &str, op_path: &str) -> String {
    let joined = format!(
        "{}/{}",
        base_path.trim_end_matches('/'),
        op_path.trim_start_matches('/')
    );
    if joined.len() > 1 {
        joined.trim_end_matches('/').to_owned()
    } else {
        joined
    }
}

/// Whether the read declares something to check beyond "it answered".
fn structured(op: &Operation) -> bool {
    op.response
        .as_ref()
        .is_some_and(|r| match r.format.as_str() {
            "json" => !r.required_pointers.is_empty() || !r.extract_pointer.is_empty(),
            "html" => {
                !r.ssr_json.is_empty() || r.html.is_some() || !r.required_html_fields.is_empty()
            }
            "file" => true,
            _ => false,
        })
}

/// The inputs an operation cannot be built without.
fn needs_inputs(op: &Operation) -> bool {
    op.params.values().any(|p| p.default.is_none())
        || op
            .query
            .values()
            .chain(op.body.iter().flat_map(|b| b.inputs().map(|(_, p)| p)))
            .any(|p| p.required && p.default.is_none())
}

/// Whether the operation signs in with a session (a cookie or a token the
/// browser keeps) rather than a key the person gives.
fn session_auth(spec: &Spec, op: &Operation) -> bool {
    if op.auth.is_empty() {
        return false;
    }
    spec.find_auth(&op.auth).is_some_and(|auth| {
        auth.kind == "browser"
            || ["cookie:", "storage:", "query:"]
                .iter()
                .any(|p| auth.value_from.starts_with(p))
    })
}

fn judgment(op: &Operation, effect: Effect) -> Judgment {
    Judgment {
        op: op.name.clone(),
        verdict: "kept",
        effect: if effect == Effect::Read {
            "read"
        } else {
            "change"
        },
        reason: String::new(),
        evidence_ids: Vec::new(),
        missing: Vec::new(),
        confirm_with: None,
    }
}

fn drop(mut judgment: Judgment, reason: impl Into<String>, missing: &[&str]) -> Judgment {
    judgment.verdict = "dropped";
    judgment.reason = reason.into();
    judgment.missing = missing.iter().map(|m| (*m).to_owned()).collect();
    judgment
}

/// The problems the install-time checks find in `spec`, by operation; the
/// ones about the whole definition under `""`.
pub fn problems_by_operation(spec: &Spec) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for problem in itda_policy::spec_problems(spec) {
        let owner = problem
            .field
            .strip_prefix("operations.")
            .and_then(|rest| {
                spec.operations
                    .iter()
                    .map(|op| op.name.as_str())
                    .filter(|name| rest == *name || rest.starts_with(&format!("{name}.")))
                    .max_by_key(|name| name.len())
            })
            .unwrap_or_default()
            .to_owned();
        out.entry(owner)
            .or_default()
            .push(format!("{}: {}", problem.field, problem.detail));
    }
    out
}

/// Judge every operation of `spec` against what was seen.
pub fn ground(spec: &Spec, seen: &Seen) -> Grounded {
    // A draft names its evidence through the judgment, so it is checked as
    // if every operation carried some.
    let mut checked = spec.clone();
    for op in &mut checked.operations {
        if op.evidence.trim().is_empty() {
            op.evidence = DRAFTED.into();
        }
    }
    let problems = problems_by_operation(&checked);
    let mut judgments = Vec::new();
    for op in &spec.operations {
        let mut judgment = ground_one(spec, op, seen, problems.get(&op.name));
        let mut cited = BTreeSet::new();
        judgment.evidence_ids.retain(|id| cited.insert(id.clone()));
        judgments.push(judgment);
    }
    // A kept change gets the kept read on the same path, or on the path
    // above it, as the read that could confirm it landed.
    let kept_reads: Vec<(&str, String)> = judgments
        .iter()
        .filter(|j| j.verdict == "kept" && j.effect == "read")
        .filter_map(|j| {
            let op = spec.operations.iter().find(|op| op.name == j.op)?;
            Some((op.name.as_str(), op.path.trim_end_matches('/').to_owned()))
        })
        .collect();
    for judgment in judgments
        .iter_mut()
        .filter(|j| j.verdict == "kept" && j.effect == "change")
    {
        let Some(op) = spec.operations.iter().find(|op| op.name == judgment.op) else {
            continue;
        };
        if op.confirm.is_some() {
            continue;
        }
        let path = op.path.trim_end_matches('/');
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        judgment.confirm_with = kept_reads
            .iter()
            .find(|(_, read)| read == path || (!parent.is_empty() && read == parent))
            .map(|(name, _)| (*name).to_owned());
    }
    let kept: Vec<&Operation> = spec
        .operations
        .iter()
        .filter(|op| {
            judgments
                .iter()
                .any(|j| j.op == op.name && j.verdict == "kept")
        })
        .collect();
    let no_actions = (!kept.is_empty()
        && kept.iter().all(|op| {
            let base = origin_and_path(spec.operation_base(op))
                .map(|(_, path)| path)
                .unwrap_or_default();
            matches!(op.method.as_str(), "" | "GET")
                && matches!(full_path(&base, &op.path).as_str(), "" | "/")
                && !structured(op)
        }))
    .then(|| {
        "every operation left reads the home page and checks nothing in its answer".to_owned()
    });
    Grounded {
        judgments,
        no_actions,
    }
}

fn ground_one(
    spec: &Spec,
    op: &Operation,
    seen: &Seen,
    problems: Option<&Vec<String>>,
) -> Judgment {
    let mut classified = op.clone();
    if classified.evidence.trim().is_empty() {
        classified.evidence = DRAFTED.into();
    }
    let declared = operation_effect(&classified, &BTreeMap::new());
    let mut out = judgment(op, declared);
    if op.effect == Effect::Read && declared != Effect::Read {
        out.reason =
            "declared a read, but its method, name, path or inputs make it a change".into();
    } else if op.effect == Effect::Unknown {
        out.reason =
            "declares no effect, so it is kept as a change; say effect: read if it only reads"
                .into();
    }
    if let Some(problems) = problems {
        return drop(out, format!("invalid: {}", problems.join("; ")), &[]);
    }
    // A read is read once more as it would be sent, when it can be built
    // without inputs: a rendered request that asks for a change is one.
    let mut effect = declared;
    if declared == Effect::Read && !needs_inputs(op) {
        match spec.build_request(op, &BTreeMap::new()) {
            Ok((url, body)) => {
                let form = op.body.as_ref().is_some_and(|b| !b.form.is_empty());
                let rendered = url::Url::parse(&url)
                    .map(|url| rendered_request_effect(&url, body.as_deref(), form));
                match rendered {
                    Ok(rendered) if rendered.effect == Effect::Write => {
                        effect = Effect::Write;
                        out.effect = "change";
                        out.reason = format!("declared a read, but {}", rendered.reason);
                    }
                    Ok(rendered) if rendered.effect == Effect::Unknown => {
                        return drop(
                            out,
                            format!("effect_unknown: {}", rendered.reason),
                            &["a request whose effect itda can tell"],
                        );
                    }
                    Ok(_) => {}
                    Err(_) => return drop(out, "the address it builds is not a URL", &[]),
                }
            }
            Err(error) => return drop(out, format!("cannot build: {}", error.msg), &[]),
        }
    }
    let base = spec.operation_base(op);
    let Some((origin, base_path)) = origin_and_path(base) else {
        return drop(out, "base_url is not a URL", &[]);
    };
    // (a) A server the site named or answered from.
    let server = seen.servers.iter().find(|server| {
        origin_and_path(&server.url).is_some_and(|(o, p)| {
            o == origin && (base_path.starts_with(&p) || p.starts_with(&base_path))
        })
    });
    let Some(server) = server else {
        return drop(
            out,
            format!("base_url {base} is not a server the site documented or answered from"),
            &[
                "an OpenAPI servers entry, an absolute address in the docs, or a page itda read on that origin",
            ],
        );
    };
    out.evidence_ids.push(server.evidence.clone());
    let method = if op.method.is_empty() {
        "GET".to_owned()
    } else {
        op.method.to_ascii_uppercase()
    };
    let path = full_path(&base_path, &op.path);
    // (b) The route, in the docs, a description or a script, or a GET of a
    // path itda saw.
    let route = seen
        .routes
        .iter()
        .filter(|route| {
            (route.method == method || (method == "HEAD" && route.method == "GET"))
                && route.origin.as_ref().is_none_or(|o| *o == origin)
                && route_matches(&route.path, &path)
        })
        .min_by_key(|route| looseness(&route.path, &path));
    let observed = (method == "GET" || method == "HEAD")
        .then(|| {
            seen.paths
                .iter()
                .find(|p| p.origin == origin && path_fits(&path, &p.path))
        })
        .flatten();
    match (route, observed) {
        (Some(route), _) => {
            // (e) A route the description says needs a sign-in itda cannot
            // use never runs.
            if route.unsupported_auth {
                out.evidence_ids.push(route.evidence.clone());
                return drop(
                    out,
                    format!("{method} {path} needs a sign-in itda cannot use"),
                    &[],
                );
            }
            out.evidence_ids.push(route.evidence.clone());
        }
        (None, Some(seen_path)) => out.evidence_ids.push(seen_path.evidence.clone()),
        (None, None) => {
            return drop(
                out,
                format!("no evidence of {method} {path}"),
                &[
                    "the route in the docs or a description, or, for a GET, a page of the site at that path",
                ],
            );
        }
    }
    // (d) A read with a session only when a path was read with one.
    if session_auth(spec, op)
        && !seen
            .paths
            .iter()
            .any(|p| p.session && p.origin == origin && path_fits(&path, &p.path))
    {
        return drop(
            out,
            format!("{method} {path} signs in, and no page was read signed in at that path"),
            &["a read of that path with the account's session"],
        );
    }
    if effect != Effect::Read {
        return out;
    }
    let Some(response) = &op.response else {
        return drop(
            out,
            "a read declares its answer: response.format and what it must hold",
            &[],
        );
    };
    let lower = path.to_ascii_lowercase();
    if FILE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        && (!response.is_file() || response.media_types.is_empty())
    {
        return drop(
            out,
            "a read that returns a file declares response.format: file with media_types",
            &[],
        );
    }
    // (c) An HTML read extracts something to check and return: records
    // (`html.items` with fields) or fields of the page (`html.page`);
    // `required_html_fields` alone returns nothing, so it never answers.
    let extracts = response.html.as_ref().is_some_and(|html| {
        (!html.items.is_empty() && !html.fields.is_empty()) || !html.page.is_empty()
    });
    if response.format == "html" && response.ssr_json.is_empty() && !extracts {
        return drop(
            out,
            "an HTML read needs an html extraction: items with fields, or page fields; required_html_fields only adds a check",
            &["a selector from the page's structure"],
        );
    }
    out
}

/// A `$result:<action>:<pointer>` reference to a value an earlier read
/// answered with: the action and the JSON pointer.
pub fn result_ref(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("$result:")?;
    let (action, pointer) = rest.split_once(':')?;
    (!action.is_empty() && (pointer.is_empty() || pointer.starts_with('/')))
        .then_some((action, pointer))
}

/// The value at `pointer` in an answer, as an input: a string, a number or a
/// boolean. An array, an object, a null or nothing there is refused, with
/// the word why.
pub fn scalar_at(answer: &Value, pointer: &str) -> Result<String, &'static str> {
    match itda_types::json::resolve_pointer(answer, pointer) {
        Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
        Some(Value::Number(number)) => Ok(number.to_string()),
        Some(Value::Bool(flag)) => Ok(flag.to_string()),
        Some(Value::Array(_) | Value::Object(_)) => Err("not_scalar"),
        Some(_) => Err("empty"),
        None => Err("missing"),
    }
}

/// Keys of an object that only say something went wrong.
const ERROR_KEYS: [&str; 7] = [
    "error",
    "errors",
    "message",
    "status",
    "code",
    "detail",
    "error_description",
];

fn filled(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

/// Whether an answer holds something: a list with records, or an object
/// with a filled key that is not only an error.
pub fn answered(data: &Value) -> bool {
    match data {
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => {
            map.values().any(filled)
                && map
                    .keys()
                    .any(|key| !ERROR_KEYS.contains(&key.to_ascii_lowercase().as_str()))
        }
        _ => false,
    }
}

/// A key that says where the next page is: there only while another page
/// is, so no check may require it.
fn page_marker(key: &str) -> bool {
    let plain = key.to_ascii_lowercase().replace(['-', '_'], "");
    itda_spec::marker_name(key)
        || matches!(
            plain.as_str(),
            "continue" | "next" | "nexturl" | "nextlink" | "prev" | "previous" | "links" | "paging"
        )
}

/// The pointers an answer's top level gives a response check: `/<key>` for
/// every filled key of an object but the ones that say where the next page
/// is. A list gets none, since an empty list is an answer too, and so does
/// an object keyed by data (languages, ids, dates) rather than by field
/// names. Empty when the answer holds nothing.
pub fn top_pointers(data: &Value) -> Vec<String> {
    if !answered(data) {
        return Vec::new();
    }
    let Value::Object(map) = data else {
        return Vec::new();
    };
    if !map.keys().all(|key| field_name(key)) {
        return Vec::new();
    }
    map.iter()
        .filter(|(key, value)| filled(value) && !page_marker(key))
        .map(|(key, _)| format!("/{key}"))
        .take(8)
        .collect()
}

fn field_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !itda_html::shape::dynamic_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(ops: Value) -> Spec {
        let value = json!({
            "spec_version": 1,
            "site": {"name": "fixture", "base_url": "https://fixture.test"},
            "auth": [{"name": "session", "kind": "browser", "value_from": "cookie:sid"}],
            "operations": ops,
        });
        itda_spec::parse(&serde_json::to_vec(&value).unwrap_or_default()).unwrap_or_else(|e| {
            panic!("{}", e.msg);
        })
    }

    fn seen() -> Seen {
        Seen {
            servers: vec![
                SeenServer {
                    url: "https://fixture.test".into(),
                    evidence: "e1".into(),
                },
                SeenServer {
                    url: "https://api.fixture.test/v2".into(),
                    evidence: "e2".into(),
                },
            ],
            routes: vec![
                SeenRoute {
                    method: "GET".into(),
                    path: "/users/@me".into(),
                    origin: None,
                    evidence: "e3".into(),
                    unsupported_auth: false,
                },
                SeenRoute {
                    method: "GET".into(),
                    path: "/v2/items/{id}".into(),
                    origin: Some("https://api.fixture.test".into()),
                    evidence: "e4".into(),
                    unsupported_auth: false,
                },
                SeenRoute {
                    method: "GET".into(),
                    path: "/v2/admin".into(),
                    origin: None,
                    evidence: "e5".into(),
                    unsupported_auth: true,
                },
                SeenRoute {
                    method: "POST".into(),
                    path: "/api/items".into(),
                    origin: None,
                    evidence: "e6".into(),
                    unsupported_auth: false,
                },
            ],
            paths: vec![SeenPath {
                origin: "https://fixture.test".into(),
                path: "/board/42".into(),
                evidence: "e7".into(),
                session: false,
            }],
        }
    }

    fn verdict<'a>(grounded: &'a Grounded, op: &str) -> &'a Judgment {
        grounded
            .judgments
            .iter()
            .find(|j| j.op == op)
            .unwrap_or_else(|| panic!("no judgment for {op}"))
    }

    #[test]
    fn a_path_itda_saw_fits_a_template_with_words_or_ids() {
        assert!(path_fits("/3/library/{page}", "/3/library/re.html"));
        assert!(path_fits("/3/library/{module}.html", "/3/library/re.html"));
        assert!(path_fits("/n/{id}", "/n/42"));
        assert!(!path_fits("/3/library/{module}.html", "/3/library/re.txt"));
        assert!(!path_fits("/3/library/{page}", "/3/reference/re.html"));
        assert!(!path_fits("/n/{id}", "/n/42/edit"));
        assert!(!path_fits("/n/{id}.html", "/n/.html"));
    }

    #[test]
    fn the_closest_route_is_cited_and_a_drafted_file_read_is_a_read() {
        let mut seen = seen();
        for (path, evidence) in [
            ("/api/v1/crates/new", "e8"),
            ("/api/v1/crates/{name}", "e9"),
            ("/api/v1/crates/{name}/{version}/download", "e10"),
        ] {
            seen.routes.push(SeenRoute {
                method: "GET".into(),
                path: path.into(),
                origin: None,
                evidence: evidence.into(),
                unsupported_auth: false,
            });
        }
        let grounded = ground(
            &spec(json!([
                {"name": "crate", "method": "GET", "path": "/api/v1/crates/{name}", "effect": "read",
                 "params": {"name": {"type": "string", "required": true}}, "response": {"format": "json"}},
                {"name": "download", "method": "GET", "path": "/api/v1/crates/{name}/{version}/download", "effect": "read",
                 "params": {"name": {"type": "string", "required": true}, "version": {"type": "string", "required": true}},
                 "response": {"format": "file", "media_types": ["application/gzip"]}}
            ])),
            &seen,
        );
        assert_eq!(verdict(&grounded, "crate").evidence_ids, ["e1", "e9"]);
        let download = verdict(&grounded, "download");
        assert_eq!(download.verdict, "kept", "{}", download.reason);
        assert_eq!(download.effect, "read");
        let undeclared = ground(
            &spec(
                json!([{"name": "crate", "method": "GET", "path": "/api/v1/crates/{name}",
                "params": {"name": {"type": "string", "required": true}}, "response": {"format": "json"}}]),
            ),
            &seen,
        );
        let crate_read = verdict(&undeclared, "crate");
        assert_eq!(crate_read.effect, "change");
        assert!(
            crate_read.reason.contains("effect: read"),
            "{}",
            crate_read.reason
        );
    }

    #[test]
    fn routes_match_across_api_and_version_prefixes() {
        assert!(route_matches("/users/@me", "/api/v10/users/@me"));
        assert!(route_matches("/api/v10/users/@me", "/users/@me"));
        assert!(route_matches("/items/{id}", "/items/7"));
        assert!(!route_matches("/users/@me", "/admin/users/@me"));
        assert!(!route_matches("/items", "/items/{id}"));
    }

    #[test]
    fn an_operation_is_kept_only_on_evidence() {
        let grounded = ground(
            &spec(json!([
                {"name": "me", "method": "GET", "path": "/api/v10/users/@me", "effect": "read",
                 "response": {"format": "json"}},
                {"name": "item", "method": "GET", "path": "/items/{id}", "base_url": "https://api.fixture.test/v2",
                 "effect": "read", "params": {"id": {"type": "int"}}, "response": {"format": "json"}},
                {"name": "made-up", "method": "GET", "path": "/api/secret", "effect": "read",
                 "response": {"format": "json"}},
                {"name": "elsewhere", "method": "GET", "path": "/users/@me", "base_url": "https://evil.test",
                 "effect": "read", "response": {"format": "json"}},
                {"name": "admin", "method": "GET", "path": "/admin", "base_url": "https://api.fixture.test/v2",
                 "effect": "read", "response": {"format": "json"}},
                {"name": "board", "method": "GET", "path": "/board/{id}", "effect": "read",
                 "params": {"id": {"type": "int"}},
                 "response": {"format": "html", "html": {"items": "tr", "fields": {"title": {"selector": "td"}}}}},
                {"name": "board-bare", "method": "GET", "path": "/board/{n}", "effect": "read",
                 "params": {"n": {"type": "int"}}, "response": {"format": "html", "required_html_fields": [{"selector": "h1"}]}},
                {"name": "board-signed-in", "method": "GET", "path": "/board/{n}", "effect": "read", "auth": "session",
                 "params": {"n": {"type": "int"}}, "response": {"format": "html", "html": {"page": {"title": {"selector": "h1"}}}}},
                {"name": "create", "method": "POST", "path": "/api/items", "effect": "write",
                 "body": {"json": {"title": {"type": "string", "required": true}}}},
                {"name": "list", "method": "GET", "path": "/api/items", "effect": "read",
                 "response": {"format": "json"}}
            ])),
            &seen(),
        );
        let me = verdict(&grounded, "me");
        assert_eq!((me.verdict, me.effect), ("kept", "read"));
        assert_eq!(me.evidence_ids, ["e1", "e3"]);
        assert_eq!(verdict(&grounded, "item").evidence_ids, ["e2", "e4"]);
        let made_up = verdict(&grounded, "made-up");
        assert_eq!(made_up.verdict, "dropped");
        assert!(made_up.reason.contains("no evidence of GET /api/secret"));
        assert!(!made_up.missing.is_empty());
        assert!(
            verdict(&grounded, "elsewhere")
                .reason
                .contains("not a server")
        );
        assert!(
            verdict(&grounded, "admin")
                .reason
                .contains("sign-in itda cannot use")
        );
        assert_eq!(verdict(&grounded, "board").evidence_ids, ["e1", "e7"]);
        assert!(
            verdict(&grounded, "board-bare")
                .reason
                .contains("needs an html extraction")
        );
        assert!(
            verdict(&grounded, "board-signed-in")
                .reason
                .contains("no page was read signed in")
        );
        // The docs name the change but not a read of the same list.
        let create = verdict(&grounded, "create");
        assert_eq!((create.verdict, create.effect), ("kept", "change"));
        assert_eq!(create.confirm_with, None);
        assert!(
            verdict(&grounded, "list")
                .reason
                .contains("no evidence of GET /api/items")
        );
        assert!(grounded.no_actions.is_none());
    }

    #[test]
    fn a_change_gets_the_read_that_could_confirm_it() {
        let mut seen = seen();
        seen.routes.push(SeenRoute {
            method: "GET".into(),
            path: "/api/items".into(),
            origin: None,
            evidence: "e8".into(),
            unsupported_auth: false,
        });
        let grounded = ground(
            &spec(json!([
                {"name": "create", "method": "POST", "path": "/api/items", "effect": "write",
                 "body": {"json": {"title": {"type": "string", "required": true}}}},
                {"name": "list", "method": "GET", "path": "/api/items", "effect": "read",
                 "response": {"format": "json"}}
            ])),
            &Seen {
                routes: seen
                    .routes
                    .into_iter()
                    .filter(|r| r.path == "/api/items")
                    .collect(),
                ..seen
            },
        );
        let create = verdict(&grounded, "create");
        assert_eq!((create.verdict, create.effect), ("kept", "change"));
        assert_eq!(create.confirm_with.as_deref(), Some("list"));
    }

    #[test]
    fn a_declared_read_that_asks_for_a_change_is_one() {
        let mut seen = seen();
        seen.routes.push(SeenRoute {
            method: "GET".into(),
            path: "/api/items".into(),
            origin: None,
            evidence: "e8".into(),
            unsupported_auth: false,
        });
        let grounded = ground(
            &spec(json!([
                {"name": "list", "method": "GET", "path": "/api/items", "effect": "read",
                 "query": {"action": {"type": "string", "default": "delete"}},
                 "response": {"format": "json"}}
            ])),
            &seen,
        );
        let list = verdict(&grounded, "list");
        assert_eq!((list.verdict, list.effect), ("kept", "change"));
        assert!(list.reason.contains("make it a change"), "{list:?}");
    }

    #[test]
    fn the_home_page_with_nothing_to_check_is_no_actions() {
        let grounded = ground(
            &spec(json!([
                {"name": "home", "method": "GET", "path": "/", "effect": "read",
                 "response": {"format": "text"}}
            ])),
            &Seen {
                paths: vec![SeenPath {
                    origin: "https://fixture.test".into(),
                    path: "/".into(),
                    evidence: "e9".into(),
                    session: false,
                }],
                ..seen()
            },
        );
        assert!(grounded.no_actions.is_some());
        assert!(grounded.kept().is_empty());
    }

    #[test]
    fn invalid_operations_say_why() {
        let grounded = ground(
            &spec(json!([
                {"name": "list", "method": "GET", "path": "/api/items", "effect": "read",
                 "headers": {"X-HTTP-Method-Override": "DELETE"}, "response": {"format": "json"}}
            ])),
            &seen(),
        );
        let list = verdict(&grounded, "list");
        assert_eq!(list.verdict, "dropped");
        assert!(
            list.reason.starts_with("invalid: operations.list"),
            "{list:?}"
        );
    }

    #[test]
    fn result_references_take_scalars_only() {
        assert_eq!(
            result_ref("$result:list-items:/items/0/id"),
            Some(("list-items", "/items/0/id"))
        );
        assert_eq!(result_ref("$result:list-items"), None);
        assert_eq!(result_ref("plain"), None);
        let answer = json!({"items": [{"id": 7, "slug": "a", "tags": ["x"], "none": null}]});
        assert_eq!(scalar_at(&answer, "/items/0/id"), Ok("7".into()));
        assert_eq!(scalar_at(&answer, "/items/0/slug"), Ok("a".into()));
        assert_eq!(scalar_at(&answer, "/items/0/tags"), Err("not_scalar"));
        assert_eq!(scalar_at(&answer, "/items/0"), Err("not_scalar"));
        assert_eq!(scalar_at(&answer, "/items/0/none"), Err("empty"));
        assert_eq!(scalar_at(&answer, "/nope"), Err("missing"));
    }

    #[test]
    fn an_answer_counts_when_it_holds_something_other_than_an_error() {
        assert!(answered(&json!({"items": [1], "total": 1})));
        assert!(answered(&json!([{"id": 1}])));
        assert!(!answered(&json!([])));
        assert!(!answered(&json!({})));
        assert!(!answered(&json!({"error": "unauthorized", "status": 401})));
        assert!(!answered(&json!({"items": [], "next": null})));
        assert!(!answered(&json!("text")));
        assert_eq!(
            top_pointers(&json!({"items": [1], "total": 3, "next": null})),
            ["/items", "/total"]
        );
        assert!(top_pointers(&json!([{"id": 1}])).is_empty());
        assert!(top_pointers(&json!({"Ruby": 60.5, "Go Template": 2.1})).is_empty());
        assert!(top_pointers(&json!({"Shell": 100.0})).is_empty());
        assert!(top_pointers(&json!({"2026-10-23": 4, "total": 4})).is_empty());
        assert_eq!(
            top_pointers(&json!({"query": {"a": 1}, "continue": {"c": "x"}, "next_cursor": "y"})),
            ["/query"]
        );
        assert!(top_pointers(&json!({"error": "x"})).is_empty());
    }
}
