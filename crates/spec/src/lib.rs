//! Declarative SiteSpec format (v1 & v2) for turning websites into typed actions.
//!
//! Derived from Hycli `src/spec.rs` (Apache-2.0). Changes: `Effect` moved here from
//! the policy module so the spec no longer depends on policy; response checks take
//! any [`ResponseLike`] instead of the network layer's response type; HTML types
//! come from `itda-html`. The format itself is unchanged.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use itda_types::error::{self as apperr, AppResult};

/// What running an operation does to the website. `Unknown` is treated like a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Read,
    Write,
    #[default]
    Unknown,
}
impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Unknown => "unknown",
        }
    }
}

/// The parts of an HTTP response a SiteSpec can verify. Implemented by the
/// network layer; tests implement it with plain data.
pub trait ResponseLike {
    fn url(&self) -> &str;
    fn status(&self) -> u16;
    /// Case-insensitive header lookup; empty when absent.
    fn header(&self, key: &str) -> &str;
    fn body(&self) -> &[u8];
    /// 2xx and not classified as a block or challenge.
    fn successful(&self) -> bool;
    fn json(&self) -> Option<Value> {
        serde_json::from_slice(self.body()).ok()
    }
}

pub const SPEC_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    #[serde(default)]
    pub spec_version: i64,
    pub site: SiteInfo,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub auth: Vec<AuthStrategy>,
    #[serde(default)]
    pub operations: Vec<Operation>,
    #[serde(default)]
    pub models: std::collections::BTreeMap<String, Model>,
    #[serde(default)]
    pub health: Option<Health>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteInfo {
    pub name: String,
    #[serde(default)]
    pub title: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_url: String,
}
impl SiteInfo {
    pub fn source_url(&self) -> &str {
        if self.source_url.is_empty() {
            &self.base_url
        } else {
            &self.source_url
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub timeout: String,
    #[serde(default)]
    pub tls_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStrategy {
    pub name: String,
    pub kind: String, // none | header | browser | derived
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default, rename = "value_from")]
    pub value_from: String, // store:<site>/<key> | cookie:<name> | storage:<key>[#<pointer>]
    /// Exact credential destination; legacy definitions use the site's base origin.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PreflightStep {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub op: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub args: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extract: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Operation {
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub method: String,
    pub path: String,
    /// Optional evidenced server for sites whose web UI and API use different origins.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transport: String,
    #[serde(default)]
    pub auth: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preflight: Vec<PreflightStep>,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, Param>,
    #[serde(default)]
    pub query: std::collections::BTreeMap<String, Param>,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<Body>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effect: Effect,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<ResponseExpectation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResponseExpectation {
    pub format: String,
    pub required_pointers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub required_html_fields: Vec<itda_html::Field>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<itda_html::Extraction>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub strip_prefix: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ssr_json: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub extract_pointer: String,
}

impl Operation {
    pub fn graphql_query(&self, args: &std::collections::BTreeMap<String, String>) -> bool {
        let query = args.get("query").map(String::as_str).or_else(|| {
            self.query
                .get("query")
                .or_else(|| {
                    self.body
                        .as_ref()
                        .and_then(|body| body.json.get("query").or_else(|| body.form.get("query")))
                })?
                .default
                .as_ref()?
                .as_str()
        });
        query.is_some_and(|query| graphql_parser::parse_query::<&str>(query).is_ok())
    }
    pub fn parse_json_response(&self, response: &impl ResponseLike) -> Option<Value> {
        let expected = self.response.as_ref();
        if let Some(exp) = expected
            && !exp.ssr_json.is_empty()
        {
            let html = String::from_utf8_lossy(response.body());
            return itda_html::extract_ssr_json(&html, &exp.ssr_json, &exp.extract_pointer);
        }
        let mut parsed = response.json().or_else(|| {
            let text = std::str::from_utf8(response.body()).ok()?.trim_start();
            let stripped = if let Some(exp) = expected {
                if !exp.strip_prefix.is_empty() {
                    text.strip_prefix(exp.strip_prefix.as_str())
                        .map(str::trim_start)
                } else {
                    None
                }
            } else {
                None
            }
            .or_else(|| {
                for prefix in [")]}',", ")]}'", "while(1);", "for(;;);"] {
                    if let Some(rest) = text.strip_prefix(prefix) {
                        return Some(rest.trim_start());
                    }
                }
                None
            })?;
            serde_json::from_str(stripped).ok()
        })?;
        if let Some(exp) = expected
            && !exp.extract_pointer.is_empty()
        {
            parsed = itda_types::json::resolve_pointer(&parsed, &exp.extract_pointer)?.clone();
        }
        Some(parsed)
    }
    pub fn matches_response(&self, response: &impl ResponseLike) -> bool {
        if !response.successful() {
            return false;
        }
        let Some(expected) = &self.response else {
            return true;
        };
        match expected.format.as_str() {
            "json" => self.parse_json_response(response).is_some_and(|body| {
                expected
                    .required_pointers
                    .iter()
                    .all(|p| itda_types::json::resolve_pointer(&body, p).is_some())
            }),
            "html" => {
                if !expected.ssr_json.is_empty() {
                    return response.header("content-type").contains("text/html")
                        && self.parse_json_response(response).is_some_and(|body| {
                            expected
                                .required_pointers
                                .iter()
                                .all(|p| itda_types::json::resolve_pointer(&body, p).is_some())
                        });
                }
                (!expected.required_html_fields.is_empty() || expected.html.is_some())
                    && response.header("content-type").contains("text/html")
                    && !response.body().is_empty()
                    && {
                        let document =
                            itda_html::parse_document(&String::from_utf8_lossy(response.body()));
                        expected
                            .required_html_fields
                            .iter()
                            .all(|field| field.is_present(&document))
                            && expected.html.as_ref().is_none_or(|extraction| {
                                extraction
                                    .extract(
                                        &String::from_utf8_lossy(response.body()),
                                        response.url(),
                                    )
                                    .is_some()
                            })
                    }
            }
            "text" => {
                response.header("content-type").starts_with("text/") && !response.body().is_empty()
            }
            "xml" => response.header("content-type").contains("xml") && !response.body().is_empty(),
            "empty" => response.body().is_empty(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    #[serde(default = "default_param_type", rename = "type")]
    pub kind: String, // string|int|float|bool
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<Value>,
}

fn default_param_type() -> String {
    "string".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    #[serde(default, rename = "json")]
    pub json: std::collections::BTreeMap<String, Param>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_template: Option<Value>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub form: std::collections::BTreeMap<String, Param>,
}
impl Body {
    pub fn inputs(&self) -> impl Iterator<Item = (&String, &Param)> {
        self.json.iter().chain(self.form.iter())
    }
    pub fn content_type(&self) -> &'static str {
        if self.form.is_empty() {
            "application/json"
        } else {
            "application/x-www-form-urlencoded"
        }
    }
}

pub type Model = std::collections::BTreeMap<String, Field>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    #[serde(default)]
    pub path: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub op: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub field: String,
    pub detail: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remedy: String,
}

pub fn parse(data: &[u8]) -> AppResult<Spec> {
    let mut s: Spec = serde_yaml::from_slice(data).map_err(|e| {
        apperr::spec(
            format!("Failed to parse SiteSpec YAML ({e})"),
            "Check YAML syntax.",
        )
    })?;
    if s.spec_version == 0 {
        s.spec_version = SPEC_VERSION;
    }
    if s.defaults.timeout.is_empty() {
        s.defaults.timeout = "15s".into();
    }
    if s.defaults.tls_profile.is_empty() {
        s.defaults.tls_profile = "go".into();
    }
    if s.auth.is_empty() {
        s.auth.push(AuthStrategy {
            name: "public".into(),
            kind: "none".into(),
            header: String::new(),
            prefix: String::new(),
            value_from: String::new(),
            origin: String::new(),
        });
    }
    Ok(s)
}

impl Spec {
    pub fn operation_base<'a>(&'a self, op: &'a Operation) -> &'a str {
        if op.base_url.is_empty() {
            &self.site.base_url
        } else {
            &op.base_url
        }
    }

    pub fn validate(&self) -> Vec<Issue> {
        let mut out: Vec<Issue> = Vec::new();
        {
            fn add(out: &mut Vec<Issue>, field: &str, detail: String, remedy: &str) {
                out.push(Issue {
                    field: field.into(),
                    detail,
                    remedy: remedy.into(),
                });
            }

            if self.spec_version != SPEC_VERSION && self.spec_version != 2 {
                add(
                    &mut out,
                    "spec_version",
                    format!("supported: 1|2, got: {}", self.spec_version),
                    "Set spec_version to 1 or 2",
                );
            }
            if self.site.name.is_empty() {
                add(
                    &mut out,
                    "site.name",
                    "empty".into(),
                    "Site identifier is required",
                );
            }
            if self.site.base_url.is_empty() {
                add(
                    &mut out,
                    "site.base_url",
                    "empty".into(),
                    "Provide an https:// absolute URL",
                );
            } else if !self.site.base_url.starts_with("http") {
                add(
                    &mut out,
                    "site.base_url",
                    "missing http(s) scheme".into(),
                    "",
                );
            }

            let mut auths = std::collections::HashSet::new();
            for a in &self.auth {
                if !auths.insert(a.name.clone()) {
                    add(
                        &mut out,
                        &format!("auth.{}", a.name),
                        "duplicate auth strategy name".into(),
                        "",
                    );
                }
                match a.kind.as_str() {
                    "none" | "browser" => {}
                    "header" => {
                        if a.header.is_empty() {
                            add(
                                &mut out,
                                &format!("auth.{}.header", a.name),
                                "missing header name for kind=header".into(),
                                "",
                            );
                        }
                        if !a.value_from.starts_with("store:") {
                            add(
                                &mut out,
                                &format!("auth.{}.value_from", a.name),
                                "must reference store:<site>/<key>".into(),
                                "Never embed raw secrets in a SiteSpec",
                            );
                        }
                    }
                    "derived" => {
                        if a.header.is_empty() {
                            add(
                                &mut out,
                                &format!("auth.{}.header", a.name),
                                "missing header name for kind=derived".into(),
                                "",
                            );
                        }
                        if !["cookie:", "storage:", "header:"]
                            .iter()
                            .any(|p| a.value_from.starts_with(p))
                        {
                            add(
                                &mut out,
                                &format!("auth.{}.value_from", a.name),
                                "must reference cookie:<name>, storage:<key>[#<pointer>], or header:<name>".into(),
                                "Example: cookie:ct0",
                            );
                        }
                    }
                    other => add(
                        &mut out,
                        &format!("auth.{}.kind", a.name),
                        format!("unsupported auth kind: {other}"),
                        "none|header|browser|derived",
                    ),
                }
            }
            let models: std::collections::HashSet<&String> = self.models.keys().collect();
            let op_names: std::collections::HashSet<&String> =
                self.operations.iter().map(|o| &o.name).collect();

            for op in &self.operations {
                let id = format!("operations.{}", op.name);
                if op.method.is_empty() {
                    add(
                        &mut out,
                        &format!("{id}.method"),
                        "empty".into(),
                        "GET|POST|PUT|PATCH|DELETE",
                    );
                }
                if !op.path.starts_with('/') {
                    add(
                        &mut out,
                        &format!("{id}.path"),
                        format!("must start with /: {}", op.path),
                        "",
                    );
                }
                if !["", "http", "browser-fetch", "browser"].contains(&op.transport.as_str()) {
                    add(
                        &mut out,
                        &format!("{id}.transport"),
                        format!("unsupported transport: {}", op.transport),
                        "http|browser-fetch|browser",
                    );
                }
                if !op.auth.is_empty() && !auths.contains(&op.auth) {
                    add(
                        &mut out,
                        &format!("{id}.auth"),
                        format!("undefined auth strategy: {}", op.auth),
                        "Define in auth block",
                    );
                }
                if !op.model.is_empty() && !models.contains(&op.model) {
                    add(
                        &mut out,
                        &format!("{id}.model"),
                        format!("undefined model: {}", op.model),
                        "Define in models block",
                    );
                }
                for step in &op.preflight {
                    let valid_target = (!step.op.is_empty() && op_names.contains(&step.op))
                        || (!step.path.is_empty() && step.path.starts_with('/'));
                    if step.name.is_empty() || !valid_target {
                        add(
                            &mut out,
                            &format!("{id}.preflight"),
                            format!("invalid preflight step: {}", step.name),
                            "Reference a defined operation or a path starting with '/'",
                        );
                    }
                }
                let path_names = path_vars(&op.path);
                for n in &path_names {
                    if !op.params.contains_key(n) {
                        add(
                            &mut out,
                            &format!("{id}.params"),
                            format!("undeclared path parameter {{{n}}}"),
                            "Declare in params",
                        );
                    }
                }
                for n in op.params.keys() {
                    if !path_names.contains(n) {
                        add(
                            &mut out,
                            &format!("{id}.params.{n}"),
                            "parameter not present in path template".into(),
                            &format!("Use {{{n}}} in path or move to query"),
                        );
                    }
                }
                let mut counts: std::collections::BTreeMap<&String, usize> = Default::default();
                for k in op.params.keys() {
                    *counts.entry(k).or_default() += 1;
                }
                for k in op.query.keys() {
                    *counts.entry(k).or_default() += 1;
                }
                if let Some(b) = &op.body {
                    if (!b.json.is_empty() || b.json_template.is_some()) && !b.form.is_empty() {
                        add(
                            &mut out,
                            &format!("{id}.body"),
                            "Choose one body encoding".into(),
                            "Use body.json/json_template or body.form, not both.",
                        );
                    }
                    for (k, _) in b.inputs() {
                        *counts.entry(k).or_default() += 1;
                    }
                }
                for (n, c) in &counts {
                    if *c > 1 {
                        add(
                            &mut out,
                            id.as_str(),
                            format!("input name conflict: {n}"),
                            "Keep input names unique across params/query/body",
                        );
                    }
                }
                let check = |where_: &str,
                             ps: &std::collections::BTreeMap<String, Param>,
                             out: &mut Vec<Issue>| {
                    for (n, p) in ps {
                        match p.kind.as_str() {
                            "string" | "int" | "float" | "bool" | "json" => {}
                            other => out.push(Issue {
                                field: format!("{where_}.{n}.type"),
                                detail: format!("unsupported type: {other}"),
                                remedy: "string|int|float|bool|json".into(),
                            }),
                        }
                        if p.required && p.default.is_some() {
                            out.push(Issue {
                                field: format!("{where_}.{n}"),
                                detail: "required parameter cannot also define a default".into(),
                                remedy: "Use required or default, not both".into(),
                            });
                        }
                    }
                };
                check(&format!("{id}.params"), &op.params, &mut out);
                check(&format!("{id}.query"), &op.query, &mut out);
                if let Some(b) = &op.body {
                    check(&format!("{id}.body.json"), &b.json, &mut out);
                    check(&format!("{id}.body.form"), &b.form, &mut out);
                }
            }
        }
        out
    }

    pub fn find_auth(&self, name: &str) -> Option<&AuthStrategy> {
        self.auth.iter().find(|a| a.name == name)
    }

    pub fn op(&self, name: &str) -> AppResult<&Operation> {
        self.operations
            .iter()
            .find(|o| o.name == name)
            .ok_or_else(|| {
                let names: Vec<&str> = self.operations.iter().map(|o| o.name.as_str()).collect();
                apperr::spec(
                    format!("Unknown operation: {name}"),
                    format!("Available: {}", names.join(", ")),
                )
            })
    }

    /// Build the request URL and body from operation definition and input arguments.
    pub fn build_request(
        &self,
        op: &Operation,
        args: &std::collections::BTreeMap<String, String>,
    ) -> AppResult<(String, Option<Vec<u8>>)> {
        let mut path = interpolate_path_template(&op.path, args);
        for n in path_vars(&op.path) {
            let p = op
                .params
                .get(&n)
                .ok_or_else(|| apperr::spec(format!("Undeclared path parameter: {n}"), ""))?;
            let mut v = coerce_str(p, &n, args, true)?;
            if v.contains("{{") {
                v = interpolate_template_str(&v, &std::collections::BTreeMap::new(), args);
            }
            path = path.replace(&format!("{{{n}}}"), &urlencode_path(&v));
        }
        let mut url_s = format!(
            "{}/{}",
            self.operation_base(op).trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let empty_resolved = std::collections::BTreeMap::new();
        let mut q = FormEncoder::default();
        for (name, p) in &op.query {
            let present = args.contains_key(name);
            if !present && p.default.is_none() {
                if p.required {
                    return Err(apperr::usage(
                        format!("Missing required input: {name}"),
                        "Provide the input shown by describe.",
                    ));
                }
                continue;
            }
            let mut v = coerce_str(p, name, args, true)?;
            if v.contains("{{") {
                v = interpolate_template_str(&v, &empty_resolved, args);
            }
            q.append_pair(name, &v);
        }
        let qs = q.finish();
        if !qs.is_empty() {
            url_s.push('?');
            url_s.push_str(&qs);
        }

        let mut body = None;
        if let Some(b) = &op.body {
            if !b.form.is_empty() {
                let mut form = FormEncoder::default();
                for (name, p) in &b.form {
                    if !args.contains_key(name) && p.default.is_none() {
                        if p.required {
                            return Err(apperr::usage(
                                format!("Missing required input: {name}"),
                                "Use the action's --help for its inputs.",
                            ));
                        }
                        continue;
                    }
                    let mut v = coerce_str(p, name, args, true)?;
                    if v.contains("{{") {
                        v = interpolate_template_str(&v, &empty_resolved, args);
                    }
                    form.append_pair(name, &v);
                }
                body = Some(form.finish().into_bytes());
            } else if let Some(template) = &b.json_template {
                let mut resolved_values = std::collections::BTreeMap::new();
                for (name, p) in &b.json {
                    let present = args.contains_key(name);
                    if !present && p.default.is_none() {
                        if p.required {
                            return Err(apperr::spec(
                                format!("Missing required argument: --{name}"),
                                format!("Example: --{name} <{}>", p.kind),
                            ));
                        }
                        continue;
                    }
                    resolved_values.insert(name.clone(), coerce_value(p, name, args)?);
                }
                let rendered = render_json_template(template, &resolved_values, args);
                body = Some(serde_json::to_vec(&rendered).unwrap_or_default());
            } else if !b.json.is_empty() {
                let mut obj = Map::new();
                for (name, p) in &b.json {
                    let present = args.contains_key(name);
                    if !present && p.default.is_none() {
                        if p.required {
                            return Err(apperr::spec(
                                format!("Missing required argument: --{name}"),
                                format!("Example: --{name} <{}>", p.kind),
                            ));
                        }
                        continue;
                    }
                    let mut val = coerce_value(p, name, args)?;
                    if let Value::String(s) = &val
                        && s.contains("{{")
                    {
                        val = Value::String(interpolate_template_str(s, &empty_resolved, args));
                    }
                    obj.insert(name.clone(), val);
                }
                body = Some(serde_json::to_vec(&Value::Object(obj)).unwrap_or_default());
            }
        }
        Ok((url_s, body))
    }

    /// Project a raw JSON response into a declared model shape with type coercion.
    pub fn apply_model(&self, name: &str, raw: &[u8]) -> AppResult<Value> {
        let model = match self.models.get(name) {
            Some(m) => m,
            None => return Ok(serde_json::from_slice(raw).unwrap_or(Value::Null)),
        };
        let doc: Value = serde_json::from_slice(raw).map_err(|e| {
            apperr::spec(
                format!("Failed to parse JSON while applying model ({e})"),
                "",
            )
        })?;
        let mut out = Map::new();
        for (field, f) in model {
            let path = if f.path.is_empty() {
                field.clone()
            } else {
                f.path.clone()
            };
            if let Some(v) = walk(&doc, path.split('.')) {
                out.insert(field.clone(), convert_type(v, &f.kind));
            }
        }
        Ok(Value::Object(out))
    }
}

fn walk<'a>(doc: &'a Value, path: impl Iterator<Item = &'a str>) -> Option<&'a Value> {
    let mut cur = doc;
    for seg in path {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

fn convert_type(v: &Value, t: &str) -> Value {
    match t {
        "string" => Value::String(match v {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            other => other.to_string(),
        }),
        "int" => v.as_i64().map(Value::from).unwrap_or(Value::Null),
        "float" => v.as_f64().map(Value::from).unwrap_or(Value::Null),
        _ => v.clone(),
    }
}

fn path_vars(path: &str) -> Vec<String> {
    let mut stripped = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(start) = rest.find("{{") {
        stripped.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("}}") {
            rest = &after[end + 2..];
        } else {
            stripped.push_str(&rest[start..]);
            rest = "";
        }
    }
    stripped.push_str(rest);

    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_var = false;
    for c in stripped.chars() {
        match c {
            '{' => {
                in_var = true;
                cur.clear();
            }
            '}' => {
                in_var = false;
                if !cur.is_empty() {
                    out.push(cur.clone());
                }
            }
            _ if in_var => cur.push(c),
            _ => {}
        }
    }
    out
}

fn interpolate_path_template(
    template: &str,
    raw_args: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            let var = after_start[..end].trim();
            if let Some(raw) = raw_args.get(var) {
                out.push_str(&urlencode_path(raw));
            }
            rest = &after_start[end + 2..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn coerce_str(
    p: &Param,
    name: &str,
    args: &std::collections::BTreeMap<String, String>,
    allow_default: bool,
) -> AppResult<String> {
    match args.get(name) {
        Some(v) => {
            check_type(p, name, v)?;
            Ok(v.clone())
        }
        None => {
            if let Some(d) = &p.default
                && allow_default
            {
                return Ok(value_to_plain_string(d));
            }
            if p.required {
                return Err(apperr::spec(
                    format!("Missing required argument: --{name}"),
                    format!("Example: --{name} <{}>", p.kind),
                ));
            }
            Ok(String::new())
        }
    }
}

fn coerce_value(
    p: &Param,
    name: &str,
    args: &std::collections::BTreeMap<String, String>,
) -> AppResult<Value> {
    match args.get(name) {
        Some(v) => {
            check_type(p, name, v)?;
            Ok(match p.kind.as_str() {
                "int" => v
                    .parse::<i64>()
                    .map(Value::from)
                    .map_err(|e| apperr::spec(format!("Failed to parse --{name}: {e}"), ""))?,
                "float" => v
                    .parse::<f64>()
                    .map(Value::from)
                    .map_err(|e| apperr::spec(format!("Failed to parse --{name}: {e}"), ""))?,
                "bool" => v
                    .parse::<bool>()
                    .map(Value::from)
                    .map_err(|e| apperr::spec(format!("Failed to parse --{name}: {e}"), ""))?,
                "json" => serde_json::from_str(v).map_err(|_| {
                    apperr::usage(
                        format!("Invalid structured input: {name}"),
                        "Provide valid JSON.",
                    )
                })?,
                _ => Value::String(v.clone()),
            })
        }
        None => p
            .default
            .clone()
            .ok_or_else(|| apperr::spec(format!("Missing required argument: --{name}"), "")),
    }
}

fn check_type(p: &Param, name: &str, v: &str) -> AppResult<()> {
    let ok = match p.kind.as_str() {
        "int" => v.parse::<i64>().is_ok(),
        "float" => v.parse::<f64>().is_ok_and(|n| n.is_finite()),
        "bool" => v.parse::<bool>().is_ok(),
        "json" => serde_json::from_str::<Value>(v).is_ok(),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(apperr::spec(
            format!("--{name} must be {}: {v}", p.kind),
            "",
        ))
    }
}

fn value_to_plain_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn urlencode_path(s: &str) -> String {
    if s == "." {
        return "%2E".into();
    }
    if s == ".." {
        return "%2E%2E".into();
    }
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Minimal form encoder without external crate dependencies.
#[derive(Default)]
pub struct FormEncoder(String);

impl FormEncoder {
    pub fn append_pair(&mut self, k: &str, v: &str) {
        if !self.0.is_empty() {
            self.0.push('&');
        }
        self.0.push_str(&form_encode(k));
        self.0.push('=');
        self.0.push_str(&form_encode(v));
    }
    pub fn finish(self) -> String {
        self.0
    }
}

fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'*' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn render_json_template(
    template: &Value,
    resolved_values: &std::collections::BTreeMap<String, Value>,
    raw_args: &std::collections::BTreeMap<String, String>,
) -> Value {
    match template {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        render_json_template(v, resolved_values, raw_args),
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| render_json_template(v, resolved_values, raw_args))
                .collect(),
        ),
        Value::String(s) => {
            if let Some(var) = s
                .strip_prefix("{{")
                .and_then(|rest| rest.strip_suffix("}}"))
            {
                let var = var.trim();
                if !var.contains("{{") {
                    if let Some(val) = resolved_values.get(var) {
                        return val.clone();
                    }
                    if let Some(raw) = raw_args.get(var) {
                        return Value::String(raw.clone());
                    }
                }
            }
            Value::String(interpolate_template_str(s, resolved_values, raw_args))
        }
        other => other.clone(),
    }
}

pub fn interpolate_template_str(
    template: &str,
    resolved_values: &std::collections::BTreeMap<String, Value>,
    raw_args: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            let var = after_start[..end].trim();
            if let Some(raw) = raw_args.get(var) {
                out.push_str(raw);
            } else if let Some(val) = resolved_values.get(var) {
                out.push_str(&value_to_plain_string(val));
            }
            rest = &after_start[end + 2..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}
