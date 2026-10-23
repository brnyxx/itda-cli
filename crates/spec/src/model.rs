//! SiteSpec data types and the YAML/JSON parser.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Effect, SPEC_VERSION};
use itda_types::error::{self as apperr, AppResult};

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
    /// The least time between two requests to the site (`2s`, `500ms`). itda
    /// never goes faster than its own pace; this can only ask for more room.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub interval: String,
    /// Ignored. Kept so v1 specs that set it still parse; requests never imitate
    /// another client's TLS fingerprint.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tls_profile: String,
}
impl Defaults {
    /// `interval` as a duration, or `None` when unset or unreadable.
    pub fn interval(&self) -> Option<std::time::Duration> {
        duration(&self.interval)
    }
}

/// Read a time written as `500ms`, `2s` or `1.5s`.
pub fn duration(text: &str) -> Option<std::time::Duration> {
    let text = text.trim();
    let (number, unit) = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map(|at| text.split_at(at))?;
    let number: f64 = number
        .parse()
        .ok()
        .filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    match unit.trim() {
        "ms" => Some(std::time::Duration::from_millis(number as u64)),
        "s" => Some(std::time::Duration::from_secs_f64(number)),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStrategy {
    pub name: String,
    pub kind: String, // none | header | browser
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default, rename = "value_from")]
    pub value_from: String, // store:<site>/<key> | storage:<key> | cookie:<name> | query:<key>
    /// Exact credential destination; legacy definitions use the site's base origin.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// The site's own sign-in form, for `itda login <site>` without a
    /// browser: a `browser` credential signed in by posting it as itda.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<LoginForm>,
}

/// Where a site's sign-in form is and what its user and password fields are
/// called. The form's action and hidden fields are read from the page each
/// time, so a one-time token in it is always fresh.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginForm {
    /// The page with the form, relative to the site.
    pub path: String,
    /// The name of the field the user name goes in.
    pub user: String,
    /// The name of the password field.
    pub password: String,
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
    /// How a read that lists things goes on to its next page; see
    /// [`Pagination`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    /// For a change: the read that shows it landed, run once when the
    /// change's answer is lost; see [`Confirm`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

/// The read that confirms a change landed. When a change is sent and its
/// answer is lost (`action_outcome_unknown`), itda runs this read once, as a
/// read, and says whether the record is there. It never resends the change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Confirm {
    /// A read in the same definition whose answer lists the record, run
    /// with its defaults and no other inputs.
    pub read: String,
    /// How to spot the record in the read's answer.
    #[serde(rename = "match")]
    pub matching: Match,
}

/// One value in the read's answer compared, as text, with one input of the
/// change: `{pointer: /items/*/title, from: body.title}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Match {
    /// A JSON pointer into the read's answer as itda returns it (after
    /// `extract_pointer` and `model`); `*` stands for every element of a
    /// list.
    pub pointer: String,
    /// The change's input the value must equal: `body.title`, `query.q` or
    /// `params.id`.
    pub from: String,
}

impl Match {
    /// Where the input is declared (`params`, `query` or `body`) and its
    /// name, when `from` has that shape.
    pub fn input(&self) -> Option<(&str, &str)> {
        let (place, name) = self.from.split_once('.')?;
        (matches!(place, "params" | "query" | "body") && !name.is_empty()).then_some((place, name))
    }
}

/// How a read turns pages. The marker for the next page is found in the
/// answer (`next`) and sent back in one input (`send`); an agent only ever
/// sees the opaque handle the runtime issues for it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pagination {
    /// `page` (a page number), `offset` (how many records to skip) or
    /// `cursor` (a marker the site hands out).
    pub style: String,
    /// Where the next marker is in the answer: a JSON pointer such as
    /// `/meta/next_cursor`, or `header:link` for the `rel="next"` address in
    /// the `Link` header. A page or offset read may leave it out: the next
    /// page is then the one after the current when the answer had records.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub next: String,
    /// A JSON pointer that must hold `true` for another page to exist, such
    /// as `/pageInfo/hasNextPage`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub more: String,
    /// The input the marker is sent back in.
    pub send: Send,
}

/// The input a pagination marker goes back in: a `query` input or a
/// `body.json` field, named.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Send {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub query: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
}

impl Pagination {
    /// The input the marker goes back in, and whether it is a body field.
    pub fn input(&self) -> (&str, bool) {
        if self.send.body.is_empty() {
            (&self.send.query, false)
        } else {
            (&self.send.body, true)
        }
    }

    /// A cursor comes from the site's previous answer, never from an agent.
    pub fn hides_input(&self) -> bool {
        self.style == "cursor"
    }
}

/// Input names that carry a page marker and may look like a token: a site's
/// `page_token` is its place in a list, not a credential.
pub const MARKER_NAMES: &[&str] = &[
    "cursor",
    "after",
    "before",
    "page",
    "offset",
    "from",
    "start",
    "skip",
    "pagetoken",
    "nextpagetoken",
    "nexttoken",
    "nextcursor",
    "endcursor",
    "continuation",
    "continuationtoken",
    "startingafter",
];

/// `next_page_token` and `nextPageToken` are the same marker name.
pub fn marker_name(name: &str) -> bool {
    let plain = name.to_ascii_lowercase().replace(['-', '_'], "");
    MARKER_NAMES.contains(&plain.as_str())
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
    /// The declared input `name`, wherever it is declared.
    pub fn input(&self, name: &str) -> Option<&Param> {
        self.params
            .get(name)
            .or_else(|| self.query.get(name))
            .or_else(|| {
                self.body
                    .as_ref()
                    .and_then(|body| body.inputs().find(|(n, _)| *n == name).map(|(_, p)| p))
            })
    }

    /// The input a pagination marker goes back in, when the read turns pages.
    pub fn pagination_input(&self) -> Option<&str> {
        self.pagination.as_ref().map(|p| p.input().0)
    }

    /// Inputs an agent may not set: filled by a preflight step, or the cursor
    /// a read's previous answer handed out.
    pub fn hidden_input(&self, name: &str) -> bool {
        self.pagination
            .as_ref()
            .is_some_and(|p| p.hides_input() && p.input().0 == name)
    }

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
        query.is_some_and(|query| {
            !crate::graphql_too_deep(query) && graphql_parser::parse_query::<&str>(query).is_ok()
        })
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

impl Param {
    /// Filled from a preflight step (`default: "{{step.name}}"`), never by
    /// the agent.
    pub fn from_step(&self) -> bool {
        self.default
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim_start().starts_with("{{step."))
    }
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
    let mut s: Spec = serde_norway::from_slice(data).map_err(|e| {
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
    if s.auth.is_empty() {
        s.auth.push(AuthStrategy {
            name: "public".into(),
            kind: "none".into(),
            header: String::new(),
            prefix: String::new(),
            value_from: String::new(),
            origin: String::new(),
            login: None,
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

    pub fn find_auth(&self, name: &str) -> Option<&AuthStrategy> {
        self.auth.iter().find(|a| a.name == name)
    }

    pub fn op(&self, name: &str) -> AppResult<&Operation> {
        self.operations
            .iter()
            .find(|o| o.name == name)
            .ok_or_else(|| {
                let names: Vec<&str> = self.operations.iter().map(|o| o.name.as_str()).collect();
                let remedy = match itda_types::util::nearest(name, names.iter().copied()) {
                    Some(close) => format!("Did you mean {close}? Available: {}", names.join(", ")),
                    None => format!("Available: {}", names.join(", ")),
                };
                apperr::spec(format!("Unknown operation: {name}"), remedy)
            })
    }
}
