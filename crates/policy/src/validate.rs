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

pub fn validate_spec(sp: &Spec) -> AppResult<()> {
    if !util::valid_id(&sp.site.name) || !sp.validate().is_empty() || sp.operations.len() > 200 {
        return Err(AppError::api("data_invalid", 400));
    }
    let base = url::Url::parse(&sp.site.base_url).map_err(|_| AppError::api("bad_url", 400))?;
    if !sp.site.source_url.is_empty() {
        validate_url(&sp.site.source_url)?;
    }
    if !["https", "http"].contains(&base.scheme())
        || !base.username().is_empty()
        || base.password().is_some()
        || base.host_str().is_none()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(AppError::api("bad_url", 400));
    }
    let mut names = std::collections::HashSet::new();
    for op in &sp.operations {
        if !["", "http", "browser-fetch", "browser"].contains(&op.transport.as_str())
            || (op.transport == "browser"
                && (op.method != "GET"
                    || op.body.is_some()
                    || operation_effect(op, &Default::default()) != Effect::Read
                    || op
                        .response
                        .as_ref()
                        .is_none_or(|response| response.format != "html")
                    || sp
                        .find_auth(&op.auth)
                        .is_some_and(|auth| auth.kind == "header")))
        {
            return Err(AppError::api("data_invalid", 400));
        }
        let destination = validate_url(sp.operation_base(op))?;
        if destination.query().is_some() || destination.fragment().is_some() {
            return Err(AppError::api("bad_url", 400));
        }
        if let Some(auth) = sp.find_auth(&op.auth) {
            if auth.kind == "header" {
                let scope = validate_url(if auth.origin.is_empty() {
                    &sp.site.base_url
                } else {
                    &auth.origin
                })?;
                if destination.origin() != scope.origin() {
                    return Err(AppError::api("unsafe_target", 400));
                }
            } else if matches!(auth.kind.as_str(), "browser" | "derived") {
                let scope = validate_url(if auth.origin.is_empty() {
                    sp.site.source_url()
                } else {
                    &auth.origin
                })?;
                if !same_site_origin(&destination, &scope) {
                    return Err(AppError::api("unsafe_target", 400));
                }
            }
        }
        if let Some(response) = &op.response
            && (!["json", "html", "text", "xml", "empty"].contains(&response.format.as_str())
                || response.required_pointers.len() > 30
                || response.strip_prefix.len() > 128
                || response.ssr_json.len() > 128
                || (!response.extract_pointer.is_empty()
                    && (!response.extract_pointer.starts_with('/')
                        || response.extract_pointer.len() > 512))
                || response
                    .html
                    .as_ref()
                    .is_some_and(|html| response.format != "html" || !html.valid())
                || response.required_html_fields.len() > 30
                || response.required_html_fields.iter().any(|field| {
                    field.attribute.len() > 80 || !itda_html::selector_valid(&field.selector)
                })
                || response
                    .required_pointers
                    .iter()
                    .any(|p| !p.is_empty() && (!p.starts_with('/') || p.len() > 512)))
        {
            return Err(AppError::api("data_invalid", 400));
        }
        if op.preflight.len() > 4 {
            return Err(AppError::api("data_invalid", 400));
        }
        for step in &op.preflight {
            if !util::valid_id(&step.name)
                || !["", "GET", "HEAD"].contains(&step.method.as_str())
                || step.path.starts_with("//")
                || step.path.contains(['\\', '\r', '\n', '#'])
            {
                return Err(AppError::api("data_invalid", 400));
            }
            if !step.base_url.is_empty() {
                let step_dest = validate_url(&step.base_url)?;
                if step_dest.query().is_some() || step_dest.fragment().is_some() {
                    return Err(AppError::api("bad_url", 400));
                }
            }
        }
        for (name, param) in op
            .params
            .iter()
            .chain(op.query.iter())
            .chain(op.body.iter().flat_map(|b| b.inputs()))
        {
            if util::secret_field(name)
                || param
                    .default
                    .as_ref()
                    .is_some_and(|v| util::redact_text(&util::plain(v)) != util::plain(v))
            {
                return Err(AppError::api("data_invalid", 400));
            }
        }
        if !util::valid_id(&op.name)
            || !names.insert(op.name.clone())
            || op.path.starts_with("//")
            || op.path.contains(['\\', '\r', '\n', '#', '?'])
            || !["GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE"]
                .contains(&op.method.as_str())
        {
            return Err(AppError::api("data_invalid", 400));
        }
        for (key, value) in sp.defaults.headers.iter().chain(op.headers.iter()) {
            let is_step_template =
                value.trim().starts_with("{{step.") && value.trim().ends_with("}}");
            if (util::secret_field(key) && !is_step_template)
                || [
                    "host",
                    "content-length",
                    "transfer-encoding",
                    "connection",
                    "proxy-authorization",
                ]
                .contains(&key.to_ascii_lowercase().as_str())
                || key.contains(['\r', '\n'])
                || value.contains(['\r', '\n'])
            {
                return Err(AppError::api("data_invalid", 400));
            }
        }
    }
    let mut credential_origins = std::collections::BTreeMap::new();
    for auth in &sp.auth {
        if !auth.origin.is_empty() {
            let scope = validate_url(&auth.origin)?;
            if scope.path() != "/" || scope.query().is_some() || scope.fragment().is_some() {
                return Err(AppError::api("bad_url", 400));
            }
        }
        if auth.kind == "header" {
            let scope = validate_url(if auth.origin.is_empty() {
                &sp.site.base_url
            } else {
                &auth.origin
            })?
            .origin()
            .ascii_serialization();
            if credential_origins
                .insert(auth.value_from.clone(), scope.clone())
                .is_some_and(|previous| previous != scope)
            {
                return Err(AppError::api("unsafe_target", 400));
            }
        }
        if auth.header.contains(['\r', '\n'])
            || auth.prefix.contains(['\r', '\n'])
            || [
                "host",
                "content-length",
                "transfer-encoding",
                "connection",
                "proxy-authorization",
            ]
            .contains(&auth.header.to_ascii_lowercase().as_str())
        {
            return Err(AppError::api("data_invalid", 400));
        }
        if auth.kind == "header"
            && !auth
                .value_from
                .starts_with(&format!("store:{}/", sp.site.name))
        {
            return Err(AppError::api("data_invalid", 400));
        }
        if auth.kind == "derived"
            && (!allowed_header(&auth.header)
                || !(auth.value_from.starts_with("cookie:")
                    || auth.value_from.starts_with("storage:")
                    || auth.value_from.starts_with("header:")))
        {
            return Err(AppError::api("data_invalid", 400));
        }
    }
    Ok(())
}
