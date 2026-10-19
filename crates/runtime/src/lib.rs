//! Runs one site action and returns a result envelope.
//!
//! This is the read path of the pipeline in the product plan: resolve the
//! action, check its inputs, refuse anything that is not a read, attach the
//! account's credential to its exact origin, send through the paced network,
//! verify the response contract, shape the data and redact secrets. Changes,
//! preflight steps and browser credentials are refused until their stages
//! exist, so nothing here can send a write.
use itda_net::{Network, Request};
use itda_policy::{Effect, operation_effect, validate_spec};
use itda_spec::{Operation, Spec};
use itda_types::{
    error::{AppError, AppResult, usage},
    url::validate_url,
    util,
};
use itda_vault::{Secret, Vault};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

/// The account used when a call names none.
pub const DEFAULT_ACCOUNT: &str = "default";

pub struct Runtime {
    net: Arc<Network>,
    vault: Arc<Vault>,
}

/// One action call. `account` is only needed for actions with credentials.
#[derive(Clone, Debug, Default)]
pub struct Call {
    pub action: String,
    pub args: BTreeMap<String, String>,
    pub account: Option<String>,
}

/// What an agent receives: the shaped data and where it came from, never the
/// credential that was used.
#[derive(Debug, Serialize)]
pub struct Envelope {
    pub site: String,
    pub action: String,
    pub effect: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub data: Value,
    pub meta: Meta,
}

#[derive(Debug, Serialize)]
pub struct Meta {
    pub http_status: u16,
    pub duration_ms: i64,
}

impl Runtime {
    pub fn new(net: Arc<Network>, vault: Arc<Vault>) -> Self {
        Self { net, vault }
    }

    pub async fn call(&self, spec: &Spec, call: Call) -> AppResult<Envelope> {
        validate_spec(spec)?;
        let op = spec.op(&call.action)?;
        check_inputs(op, &call.args)?;
        if operation_effect(op, &call.args) != Effect::Read {
            return Err(AppError::api("writes_disabled", 403));
        }
        if !matches!(op.transport.as_str(), "" | "http") || !op.preflight.is_empty() {
            return Err(usage(
                format!("{} needs a browser or preflight steps", op.name),
                "This version runs plain HTTP reads only.",
            ));
        }

        let (url, body) = spec.build_request(op, &call.args)?;
        let destination = validate_url(&url)?;
        let mut headers: Vec<(String, String)> = spec
            .defaults
            .headers
            .iter()
            .chain(op.headers.iter())
            .filter(|(_, value)| !value.contains("{{"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let (Some(_), Some(declared)) = (&body, &op.body) {
            headers.push(("content-type".into(), declared.content_type().into()));
        }

        let account = (!op.auth.is_empty()).then(|| {
            call.account
                .clone()
                .unwrap_or_else(|| DEFAULT_ACCOUNT.to_owned())
        });
        let mut secrets = Vec::new();
        if let Some(account) = &account
            && let Some((name, value, secret)) =
                self.credential(spec, op, account, &destination).await?
        {
            headers.push((name, value));
            secrets.push(secret);
        }

        let mut request = Request::read(url);
        request.method = if op.method.is_empty() {
            "GET".into()
        } else {
            op.method.clone()
        };
        request.headers = headers;
        request.body = body;
        request.account_id = account.clone().unwrap_or_default();
        let response = self.net.fetch(request).await?;

        if !response.successful() {
            return Err(refusal(&response, account.is_some()));
        }
        if !op.matches_response(&response) {
            return Err(AppError::api("response_mismatch", 502));
        }
        let mut data = shape(spec, op, &response)?;
        let exposed: Vec<String> = secrets.iter().map(|s| s.expose().to_owned()).collect();
        util::redact(&mut data, &exposed);

        Ok(Envelope {
            site: spec.site.name.clone(),
            action: op.name.clone(),
            effect: Effect::Read.as_str(),
            account,
            data,
            meta: Meta {
                http_status: response.status,
                duration_ms: response.duration_ms,
            },
        })
    }

    /// The header a `header` credential adds, only for the origin it belongs to.
    async fn credential(
        &self,
        spec: &Spec,
        op: &Operation,
        account: &str,
        destination: &url::Url,
    ) -> AppResult<Option<(String, String, Secret)>> {
        let auth = spec
            .find_auth(&op.auth)
            .ok_or_else(|| AppError::api("data_invalid", 400))?;
        match auth.kind.as_str() {
            "none" => Ok(None),
            "header" => {
                let scope = validate_url(if auth.origin.is_empty() {
                    &spec.site.base_url
                } else {
                    &auth.origin
                })?;
                if scope.origin() != destination.origin() {
                    return Err(AppError::api("unsafe_target", 400));
                }
                let name = auth
                    .value_from
                    .strip_prefix(&format!("store:{}/", spec.site.name))
                    .ok_or_else(|| AppError::api("data_invalid", 400))?;
                let secret = self.vault.secret(&spec.site.name, account, name).await?;
                let value = format!("{}{}", auth.prefix, secret.expose());
                Ok(Some((auth.header.clone(), value, secret)))
            }
            _ => Err(usage(
                format!("{} signs in through the browser", op.name),
                "This version supports API-key accounts only.",
            )),
        }
    }
}

/// Every input must be declared; required ones and types are checked when the
/// request is built.
fn check_inputs(op: &Operation, args: &BTreeMap<String, String>) -> AppResult<()> {
    for name in args.keys() {
        let declared = op.params.contains_key(name)
            || op.query.contains_key(name)
            || op
                .body
                .as_ref()
                .is_some_and(|body| body.inputs().any(|(n, _)| n == name));
        if !declared {
            return Err(usage(
                format!("Unknown input: {name}"),
                "Use only the inputs the action declares.",
            ));
        }
    }
    Ok(())
}

/// Why a response that was not a success failed. Blocks and expired sessions
/// have already paused the scope in the network layer.
fn refusal(response: &itda_net::Response, signed_in: bool) -> AppError {
    match (response.defense.as_str(), response.status) {
        ("none", 401 | 403) if signed_in => AppError::api("auth_required", 401),
        ("none", 404) => AppError::api("not_found", 404),
        ("none", 429) | ("rate-limit", _) => AppError::api("site_rate_limited", 429),
        ("none", _) => AppError::api("request_failed", 502),
        _ => AppError::api("blocked_by_site", 403),
    }
}

/// The data the response contract describes, projected through the action's
/// model when it has one.
fn shape(spec: &Spec, op: &Operation, response: &itda_net::Response) -> AppResult<Value> {
    let format = op
        .response
        .as_ref()
        .map(|r| r.format.as_str())
        .unwrap_or("json");
    match format {
        "empty" => Ok(Value::Null),
        "text" | "xml" => Ok(Value::String(
            String::from_utf8_lossy(&response.bytes).into_owned(),
        )),
        "html" if op.response.as_ref().is_some_and(|r| r.ssr_json.is_empty()) => {
            let html = String::from_utf8_lossy(&response.bytes);
            op.response
                .as_ref()
                .and_then(|r| r.html.as_ref())
                .and_then(|extraction| extraction.extract(&html, &response.url))
                .ok_or_else(|| AppError::api("response_mismatch", 502))
        }
        _ => {
            let parsed = op
                .parse_json_response(response)
                .ok_or_else(|| AppError::api("response_mismatch", 502))?;
            if op.model.is_empty() {
                Ok(parsed)
            } else {
                spec.apply_model(&op.model, &serde_json::to_vec(&parsed)?)
            }
        }
    }
}
