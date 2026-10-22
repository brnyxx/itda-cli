//! Runs one site action and returns a result envelope.
//!
//! The pipeline in the product plan: resolve the action, check its inputs,
//! attach the account's credential to its exact origin, send through the
//! paced network, verify the response contract, shape the data and redact
//! secrets.
//!
//! Reads run directly. A change runs only with an approval the user granted
//! for exactly this request: the first call records the request and returns
//! `approval_required` with an id, the user reviews and grants it outside the
//! agent (`itda approve`), and a second call with that id uses it up before
//! anything is sent. Without an approval store, changes are refused.
//! A `browser` credential is the cookie jar the user's own sign-in left
//! behind (`itda login --browser`); it is sent only to the same site.
//! Preflight steps read per-visit values such as a CSRF token from the same
//! origin just before the action is sent; see `preflight`.
//!
//! With an audit log, every call is recorded with its outcome, and a change
//! is recorded before it is sent: if that record cannot be written, the
//! change is not sent.
pub mod audit;
mod preflight;
use itda_net::{Network, Request};
use itda_policy::{
    Approvals, Effect, ExecutionRequest, ReviewInput, operation_effect, validate_spec,
};
use itda_spec::{Operation, Spec, interpolate_template_str};
use itda_types::{
    error::{AppError, AppResult, usage},
    url::{same_site_origin, validate_url},
    util,
};
use itda_vault::{Secret, Vault};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

/// The account used when a call names none.
pub const DEFAULT_ACCOUNT: &str = "default";

/// The vault secret that holds a browser sign-in: a `Cookie` header value.
pub const SESSION_SECRET: &str = "session";

pub struct Runtime {
    net: Arc<Network>,
    vault: Arc<Vault>,
    approvals: Option<Arc<Approvals>>,
    audit: Option<Arc<audit::AuditLog>>,
}

/// One action call. `account` is only needed for actions with credentials.
#[derive(Clone, Debug, Default)]
pub struct Call {
    pub action: String,
    pub args: BTreeMap<String, String>,
    pub account: Option<String>,
    /// The id of an approval the user granted for this exact change.
    pub approval: Option<String>,
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
        Self {
            net,
            vault,
            approvals: None,
            audit: None,
        }
    }

    /// Record every call in this log.
    pub fn with_audit(mut self, log: Arc<audit::AuditLog>) -> Self {
        self.audit = Some(log);
        self
    }

    /// Allow changes that the user approves through this store.
    pub fn with_approvals(mut self, approvals: Arc<Approvals>) -> Self {
        self.approvals = Some(approvals);
        self
    }

    pub async fn call(&self, spec: &Spec, call: Call) -> AppResult<Envelope> {
        let result = self.run(spec, &call).await;
        if let Some(log) = &self.audit {
            let mut event = self.event(spec, &call, "call");
            match &result {
                Ok(envelope) => {
                    event.outcome = "ok".into();
                    event.http_status = envelope.meta.http_status;
                }
                Err(error) => {
                    event.outcome = error.public_code.clone();
                    if let Some(id) = error.extra.get("approval").and_then(Value::as_str) {
                        event.approval = id.to_owned();
                    }
                }
            }
            // The call already happened; a failed record must not hide its
            // result. Changes were recorded before they were sent.
            let _ = log.append(event);
        }
        result
    }

    fn event(&self, spec: &Spec, call: &Call, kind: &'static str) -> audit::Event {
        let op = spec.op(&call.action).ok();
        audit::Event {
            kind,
            site: spec.site.name.clone(),
            action: call.action.clone(),
            effect: op
                .map(|op| operation_effect(op, &call.args).as_str().to_owned())
                .unwrap_or_default(),
            account: match (op, &call.account) {
                (_, Some(account)) => account.clone(),
                (Some(op), None) if !op.auth.is_empty() => DEFAULT_ACCOUNT.into(),
                _ => String::new(),
            },
            approval: call.approval.clone().unwrap_or_default(),
            inputs: call.args.clone(),
            ..audit::Event::default()
        }
    }

    async fn run(&self, spec: &Spec, call: &Call) -> AppResult<Envelope> {
        validate_spec(spec)?;
        let op = spec.op(&call.action)?;
        check_inputs(op, &call.args)?;
        let effect = operation_effect(op, &call.args);
        let approvals = match (effect, &self.approvals) {
            (Effect::Read, _) => None,
            (_, Some(approvals)) => Some(approvals),
            (_, None) => return Err(AppError::api("writes_disabled", 403)),
        };
        if !matches!(op.transport.as_str(), "" | "http") {
            return Err(usage(
                format!("{} needs a browser", op.name),
                "This version sends plain HTTP requests only.",
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
        let mut exposed = Vec::new();
        if let Some(account) = &account
            && let Some(credential) = self.credential(spec, op, account, &destination).await?
        {
            headers.push((credential.header, credential.value));
            exposed.extend(credential.redact);
            secrets.push(credential.secret);
        }

        let mut request = Request::read(url.clone());
        request.method = if op.method.is_empty() {
            "GET".into()
        } else {
            op.method.clone()
        };
        request.headers = headers;
        request.body = body;
        request.account_id = account.clone().unwrap_or_default();
        request.interval = spec.defaults.interval();

        if let Some(approvals) = approvals {
            let execution = ExecutionRequest {
                site_id: spec.site.name.clone(),
                action_id: op.name.clone(),
                account_id: request.account_id.clone(),
                inputs: call.args.clone(),
                spec_hash: util::hash(&serde_json::to_vec(spec)?),
                credential_hash: secrets.first().map(Secret::fingerprint).unwrap_or_default(),
            };
            match &call.approval {
                None => {
                    let review = approvals
                        .request_approval(
                            execution,
                            title(&spec.site.title, &spec.site.name),
                            title(&op.desc, &op.name),
                            request.account_id.clone(),
                            review_inputs(&request.method, &url, &call.args),
                        )
                        .await?;
                    let mut error = AppError::api("approval_required", 409)
                        .with("approval", review.id.clone())
                        .with("expires_at", review.expires_at.clone());
                    error.msg = format!(
                        "{}.{} changes the site and waits for the user's approval {}",
                        spec.site.name, op.name, review.id
                    );
                    error.remedy = format!(
                        "Ask the user to run `itda approve {}`, then call again with approval {}.",
                        review.id, review.id
                    );
                    return Err(error);
                }
                Some(id) => approvals.consume(id, &execution).await?,
            }
            if let Some(log) = &self.audit {
                let mut event = self.event(spec, call, "change_started");
                event.outcome = "sending".into();
                log.append(event)?;
            }
            // A change is sent once: no retries and no redirects.
            request.read_only = false;
            request.follow_redirects = false;
        }

        if !op.preflight.is_empty() {
            let found = preflight::run(
                &self.net,
                spec,
                op,
                &call.args,
                &request.headers,
                &request.account_id,
                &destination,
            )
            .await?;
            let mut args = call.args.clone();
            args.extend(found.clone());
            let (url, body) = spec.build_request(op, &args)?;
            if validate_url(&url)?.origin() != destination.origin() {
                return Err(AppError::api("unsafe_target", 400));
            }
            request.url = url;
            request.body = body;
            request.headers.extend(
                spec.defaults
                    .headers
                    .iter()
                    .chain(op.headers.iter())
                    .filter(|(_, value)| value.contains("{{"))
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            interpolate_template_str(v, &BTreeMap::new(), &args),
                        )
                    }),
            );
            exposed.extend(found.into_values().filter(|value| value.len() >= 8));
        }

        let read = approvals.is_none();
        let response = match self.net.fetch(request).await {
            Ok(response) => response,
            Err(error)
                if !read
                    && matches!(
                        error.public_code.as_str(),
                        "timeout" | "request_failed" | "too_large"
                    ) =>
            {
                return Err(AppError::api("action_outcome_unknown", 502));
            }
            Err(error) => return Err(error),
        };

        if !response.successful() {
            return Err(refusal(&response, account.is_some()));
        }
        if read && !op.matches_response(&response) {
            return Err(AppError::api("response_mismatch", 502));
        }
        // The change has happened, so a reply that does not match the
        // contract is reported as missing data rather than a failure.
        let mut data = match shape(spec, op, &response) {
            Ok(data) => data,
            Err(_) if !read => Value::Null,
            Err(error) => return Err(error),
        };
        exposed.extend(secrets.iter().map(|s| s.expose().to_owned()));
        util::redact(&mut data, &exposed);

        Ok(Envelope {
            site: spec.site.name.clone(),
            action: op.name.clone(),
            effect: effect.as_str(),
            account,
            data,
            meta: Meta {
                http_status: response.status,
                duration_ms: response.duration_ms,
            },
        })
    }

    /// The header a credential adds, only for the destination it belongs to:
    /// a `header` key to its exact origin, a `browser` session to its site.
    async fn credential(
        &self,
        spec: &Spec,
        op: &Operation,
        account: &str,
        destination: &url::Url,
    ) -> AppResult<Option<Credential>> {
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
                Ok(Some(Credential {
                    header: auth.header.clone(),
                    value: format!("{}{}", auth.prefix, secret.expose()),
                    redact: Vec::new(),
                    secret,
                }))
            }
            "browser" => {
                let scope = validate_url(if auth.origin.is_empty() {
                    spec.site.source_url()
                } else {
                    &auth.origin
                })?;
                if !same_site_origin(&scope, destination) {
                    return Err(AppError::api("unsafe_target", 400));
                }
                let secret = self
                    .vault
                    .secret(&spec.site.name, account, SESSION_SECRET)
                    .await?;
                let cookies = cookie_pairs(secret.expose());
                // A jar without the cookie that proves sign-in is a signed-out session.
                if let Some(required) = auth.value_from.strip_prefix("cookie:")
                    && !cookies.iter().any(|(name, _)| *name == required)
                {
                    return Err(AppError::api("auth_required", 401));
                }
                let redact = cookies
                    .iter()
                    .filter(|(_, value)| value.len() >= 8)
                    .map(|(_, value)| (*value).to_owned())
                    .collect();
                Ok(Some(Credential {
                    header: "cookie".into(),
                    value: secret.expose().to_owned(),
                    redact,
                    secret,
                }))
            }
            _ => Err(usage(
                format!("{} uses a credential this version cannot send", op.name),
                "Supported kinds are header and browser.",
            )),
        }
    }
}

/// A credential ready to send, and the values to keep out of results.
struct Credential {
    header: String,
    value: String,
    redact: Vec<String>,
    secret: Secret,
}

/// The `name=value` pairs of a `Cookie` header value.
fn cookie_pairs(jar: &str) -> Vec<(&str, &str)> {
    jar.split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .map(|(name, value)| (name.trim(), value.trim()))
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

fn title(preferred: &str, fallback: &str) -> String {
    if preferred.is_empty() {
        fallback
    } else {
        preferred
    }
    .to_owned()
}

/// What the user reviews: the exact request line and every input.
fn review_inputs(method: &str, url: &str, args: &BTreeMap<String, String>) -> Vec<ReviewInput> {
    std::iter::once(ReviewInput {
        label: "request".into(),
        value: format!("{method} {url}"),
    })
    .chain(args.iter().map(|(label, value)| ReviewInput {
        label: label.clone(),
        value: value.clone(),
    }))
    .collect()
}

/// Every input must be declared and not filled by a preflight step; required
/// ones and types are checked when the request is built.
fn check_inputs(op: &Operation, args: &BTreeMap<String, String>) -> AppResult<()> {
    for name in args.keys() {
        let declared = op
            .params
            .get(name)
            .or_else(|| op.query.get(name))
            .or_else(|| {
                op.body
                    .as_ref()
                    .and_then(|body| body.inputs().find(|(n, _)| *n == name).map(|(_, p)| p))
            })
            .is_some_and(|param| !param.from_step());
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
pub fn refusal(response: &itda_net::Response, signed_in: bool) -> AppError {
    match (response.defense.as_str(), response.status) {
        ("none", 401 | 403) if signed_in => AppError::api("auth_required", 401),
        ("none", 404) => AppError::api("not_found", 404),
        ("none", 429) | ("rate-limit", _) => {
            let mut error = AppError::api("site_rate_limited", 429);
            if let Some(wait) = itda_net::retry_after(&response.headers) {
                let seconds = wait.as_secs().max(1);
                error.msg = format!("the site asked to wait {seconds} s before the next request");
                error = error.with("retry_after", seconds);
            }
            error
        }
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
