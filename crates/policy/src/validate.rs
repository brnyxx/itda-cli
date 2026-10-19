//! Safety validation of a SiteSpec before it is installed or executed.
//!
//! Derived from Hycli `src/policy.rs` (`validate_spec`) and `src/session_recipe.rs`
//! (`allowed_header`), Apache-2.0. Changes: URL checks come from `itda_types::url`
//! and selector checks from `itda_html`.
use crate::classify::operation_effect;
use itda_spec::{Effect, Spec};
use itda_types::{
    error::{AppError, AppResult},
    url::{same_site_origin, validate_url},
    util,
};

/// Headers a derived credential may set. Transport and identity headers are refused.
pub fn allowed_header(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && ![
            "cookie",
            "host",
            "origin",
            "referer",
            "content-length",
            "transfer-encoding",
            "connection",
            "proxy-authorization",
            "proxy-connection",
        ]
        .contains(&name.to_ascii_lowercase().as_str())
        && !name.to_ascii_lowercase().starts_with("sec-")
}

/// One reason a SiteSpec is refused: a stable public code, the field it is about
/// and what is wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub code: &'static str,
    pub field: String,
    pub detail: String,
}

/// Refuse a SiteSpec that is unsafe to install. The error names the first
/// problem; [`spec_problems`] lists all of them.
pub fn validate_spec(sp: &Spec) -> AppResult<()> {
    match spec_problems(sp).into_iter().next() {
        None => Ok(()),
        Some(problem) => {
            let mut error = AppError::api(problem.code, 400);
            error.msg = format!("{}: {}", problem.field, problem.detail);
            Err(error)
        }
    }
}

const MAX_OPERATIONS: usize = 200;
const FORBIDDEN_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "proxy-authorization",
];

#[derive(Default)]
struct Problems(Vec<Problem>);

impl Problems {
    fn add(&mut self, code: &'static str, field: impl Into<String>, detail: impl Into<String>) {
        self.0.push(Problem {
            code,
            field: field.into(),
            detail: detail.into(),
        });
    }

    /// Validate a URL, recording a problem when it is refused.
    fn url(&mut self, field: &str, raw: &str) -> Option<url::Url> {
        let url = validate_url(raw).ok();
        if url.is_none() {
            self.add(
                "bad_url",
                field,
                "must be an http(s) URL without credentials or credential-named query parameters",
            );
        }
        url
    }
}

/// Every reason the SiteSpec would be refused, in the order they are found.
pub fn spec_problems(sp: &Spec) -> Vec<Problem> {
    let mut out = Problems::default();
    if !util::valid_id(&sp.site.name) {
        out.add("data_invalid", "site.name", "must be a short lower-case id");
    }
    for issue in sp.validate() {
        out.add("data_invalid", issue.field, issue.detail);
    }
    if sp.operations.len() > MAX_OPERATIONS {
        out.add(
            "data_invalid",
            "operations",
            format!("at most {MAX_OPERATIONS} operations are allowed"),
        );
    }
    match url::Url::parse(&sp.site.base_url) {
        Ok(base) => {
            if !["https", "http"].contains(&base.scheme())
                || !base.username().is_empty()
                || base.password().is_some()
                || base.host_str().is_none()
                || base.query().is_some()
                || base.fragment().is_some()
            {
                out.add(
                    "bad_url",
                    "site.base_url",
                    "must be an http(s) origin and path without credentials, query or fragment",
                );
            }
        }
        Err(_) => out.add("bad_url", "site.base_url", "is not a valid URL"),
    }
    if !sp.site.source_url.is_empty() {
        out.url("site.source_url", &sp.site.source_url);
    }
    let mut names = std::collections::HashSet::new();
    for op in &sp.operations {
        check_operation(sp, op, &mut names, &mut out);
    }
    check_auth(sp, &mut out);
    out.0
}

fn check_operation(
    sp: &Spec,
    op: &itda_spec::Operation,
    names: &mut std::collections::HashSet<String>,
    out: &mut Problems,
) {
    let id = format!("operations.{}", op.name);
    if !["", "http", "browser-fetch", "browser"].contains(&op.transport.as_str()) {
        out.add(
            "data_invalid",
            format!("{id}.transport"),
            "unknown transport",
        );
    } else if op.transport == "browser"
        && (op.method != "GET"
            || op.body.is_some()
            || operation_effect(op, &Default::default()) != Effect::Read
            || op
                .response
                .as_ref()
                .is_none_or(|response| response.format != "html")
            || sp
                .find_auth(&op.auth)
                .is_some_and(|auth| auth.kind == "header"))
    {
        out.add(
            "data_invalid",
            format!("{id}.transport"),
            "a rendered browser read must be a GET read that returns html without header auth",
        );
    }
    if let Some(destination) = out.url(&format!("{id}.base_url"), sp.operation_base(op)) {
        if destination.query().is_some() || destination.fragment().is_some() {
            out.add(
                "bad_url",
                format!("{id}.base_url"),
                "must not carry a query or fragment",
            );
        }
        check_operation_auth(sp, op, &id, &destination, out);
    }
    if let Some(response) = &op.response {
        check_response(response, &id, out);
    }
    if op.preflight.len() > 4 {
        out.add(
            "data_invalid",
            format!("{id}.preflight"),
            "at most 4 preflight steps are allowed",
        );
    }
    for step in &op.preflight {
        let field = format!("{id}.preflight.{}", step.name);
        if !util::valid_id(&step.name)
            || !["", "GET", "HEAD"].contains(&step.method.as_str())
            || step.path.starts_with("//")
            || step.path.contains(['\\', '\r', '\n', '#'])
        {
            out.add(
                "data_invalid",
                field.clone(),
                "must have an id, use GET or HEAD and a plain relative path",
            );
        }
        if !step.base_url.is_empty()
            && let Some(step_dest) = out.url(&format!("{field}.base_url"), &step.base_url)
            && (step_dest.query().is_some() || step_dest.fragment().is_some())
        {
            out.add(
                "bad_url",
                format!("{field}.base_url"),
                "must not carry a query or fragment",
            );
        }
    }
    for (name, param) in op
        .params
        .iter()
        .chain(op.query.iter())
        .chain(op.body.iter().flat_map(|b| b.inputs()))
    {
        if util::secret_field(name) {
            out.add(
                "data_invalid",
                format!("{id}.inputs.{name}"),
                "credentials must come from an auth strategy, not an input",
            );
        } else if param
            .default
            .as_ref()
            .is_some_and(|v| util::redact_text(&util::plain(v)) != util::plain(v))
        {
            out.add(
                "data_invalid",
                format!("{id}.inputs.{name}.default"),
                "looks like a secret",
            );
        }
    }
    if !util::valid_id(&op.name) || !names.insert(op.name.clone()) {
        out.add("data_invalid", id.clone(), "name must be a unique id");
    }
    if op.path.starts_with("//") || op.path.contains(['\\', '\r', '\n', '#', '?']) {
        out.add(
            "data_invalid",
            format!("{id}.path"),
            "must be a plain relative path without query or fragment",
        );
    }
    if !["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE"].contains(&op.method.as_str()) {
        out.add("data_invalid", format!("{id}.method"), "unknown method");
    }
    for (key, value) in sp.defaults.headers.iter().chain(op.headers.iter()) {
        let is_step_template = value.trim().starts_with("{{step.") && value.trim().ends_with("}}");
        if (util::secret_field(key) && !is_step_template)
            || FORBIDDEN_HEADERS.contains(&key.to_ascii_lowercase().as_str())
            || key.contains(['\r', '\n'])
            || value.contains(['\r', '\n'])
        {
            out.add(
                "data_invalid",
                format!("{id}.headers.{key}"),
                "credential, transport or multi-line headers are not allowed",
            );
        }
    }
}

fn check_operation_auth(
    sp: &Spec,
    op: &itda_spec::Operation,
    id: &str,
    destination: &url::Url,
    out: &mut Problems,
) {
    let Some(auth) = sp.find_auth(&op.auth) else {
        return;
    };
    let field = format!("{id}.auth");
    if auth.kind == "header" {
        let scope = if auth.origin.is_empty() {
            &sp.site.base_url
        } else {
            &auth.origin
        };
        if let Some(scope) = out.url(&field, scope)
            && destination.origin() != scope.origin()
        {
            out.add(
                "unsafe_target",
                field,
                "header credentials may only go to their own origin",
            );
        }
    } else if matches!(auth.kind.as_str(), "browser" | "derived") {
        let scope = if auth.origin.is_empty() {
            sp.site.source_url()
        } else {
            &auth.origin
        };
        if let Some(scope) = out.url(&field, scope)
            && !same_site_origin(destination, &scope)
        {
            out.add(
                "unsafe_target",
                field,
                "session credentials may only go to the same site",
            );
        }
    }
}

fn check_response(response: &itda_spec::ResponseExpectation, id: &str, out: &mut Problems) {
    let field = format!("{id}.response");
    if !["json", "html", "text", "xml", "empty"].contains(&response.format.as_str()) {
        out.add("data_invalid", format!("{field}.format"), "unknown format");
    }
    if response.required_pointers.len() > 30
        || response
            .required_pointers
            .iter()
            .any(|p| !p.is_empty() && (!p.starts_with('/') || p.len() > 512))
    {
        out.add(
            "data_invalid",
            format!("{field}.required_pointers"),
            "at most 30 JSON pointers starting with /",
        );
    }
    if response.strip_prefix.len() > 128 || response.ssr_json.len() > 128 {
        out.add(
            "data_invalid",
            field.clone(),
            "strip_prefix and ssr_json are limited to 128 characters",
        );
    }
    if !response.extract_pointer.is_empty()
        && (!response.extract_pointer.starts_with('/') || response.extract_pointer.len() > 512)
    {
        out.add(
            "data_invalid",
            format!("{field}.extract_pointer"),
            "must be a JSON pointer starting with /",
        );
    }
    if response
        .html
        .as_ref()
        .is_some_and(|html| response.format != "html" || !html.valid())
    {
        out.add(
            "data_invalid",
            format!("{field}.html"),
            "needs format html and valid selectors",
        );
    }
    if response.required_html_fields.len() > 30
        || response
            .required_html_fields
            .iter()
            .any(|field| field.attribute.len() > 80 || !itda_html::selector_valid(&field.selector))
    {
        out.add(
            "data_invalid",
            format!("{field}.required_html_fields"),
            "at most 30 fields with valid CSS selectors",
        );
    }
}

fn check_auth(sp: &Spec, out: &mut Problems) {
    let mut credential_origins = std::collections::BTreeMap::new();
    for auth in &sp.auth {
        let field = format!("auth.{}", auth.name);
        if !auth.origin.is_empty()
            && let Some(scope) = out.url(&format!("{field}.origin"), &auth.origin)
            && (scope.path() != "/" || scope.query().is_some() || scope.fragment().is_some())
        {
            out.add(
                "bad_url",
                format!("{field}.origin"),
                "must be a bare origin",
            );
        }
        if auth.kind == "header" {
            let scope = if auth.origin.is_empty() {
                &sp.site.base_url
            } else {
                &auth.origin
            };
            if let Some(scope) = out.url(&field, scope) {
                let scope = scope.origin().ascii_serialization();
                if credential_origins
                    .insert(auth.value_from.clone(), scope.clone())
                    .is_some_and(|previous| previous != scope)
                {
                    out.add(
                        "unsafe_target",
                        format!("{field}.value_from"),
                        "the same credential is sent to two origins",
                    );
                }
            }
        }
        if auth.header.contains(['\r', '\n'])
            || auth.prefix.contains(['\r', '\n'])
            || FORBIDDEN_HEADERS.contains(&auth.header.to_ascii_lowercase().as_str())
        {
            out.add(
                "data_invalid",
                format!("{field}.header"),
                "transport or multi-line headers are not allowed",
            );
        }
        if auth.kind == "header"
            && !auth
                .value_from
                .starts_with(&format!("store:{}/", sp.site.name))
        {
            out.add(
                "data_invalid",
                format!("{field}.value_from"),
                "header credentials must come from this site's own store",
            );
        }
        if auth.kind == "derived"
            && (!allowed_header(&auth.header)
                || !(auth.value_from.starts_with("cookie:")
                    || auth.value_from.starts_with("storage:")
                    || auth.value_from.starts_with("header:")))
        {
            out.add(
                "data_invalid",
                field,
                "derived credentials need an allowed header and a cookie, storage or header source",
            );
        }
    }
}
