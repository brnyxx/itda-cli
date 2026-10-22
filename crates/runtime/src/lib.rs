//! Runs one site action and returns a result envelope.
//!
//! The pipeline in the product plan: resolve the action, check its inputs,
//! attach the account's credential to its exact origin, send through the
//! paced network, verify the response contract, shape the data and redact
//! secrets.
//!
//! Reads run directly, after the request built from the template is read
//! once more: a read whose rendered address or body asks for a change goes
//! the way of a change, with the reason, and one whose effect cannot be told
//! is refused (`effect_unknown`). A change runs only with an approval the
//! user granted for exactly this request: the first call records the request
//! and returns `approval_required` with an id, the user reviews and grants it
//! outside the agent (`itda approve`), and a second call with that id uses it
//! up before anything is sent. Without an approval store, changes are refused.
//! A `browser` credential is the cookie jar the user's own sign-in left
//! behind (`itda login --browser`); it is sent only to the same site.
//! Preflight steps read per-visit values such as a CSRF token from the same
//! origin just before the action is sent; see `preflight`. A read that
//! declares `pagination` returns `meta.next`, a handle for the following
//! page, and continues from it with `next`; see `pagination`.
//!
//! With an audit log, every call is recorded with its outcome, and a change
//! is recorded before it is sent: if that record cannot be written, the
//! change is not sent.
//!
//! A change may name the read that shows it landed (`confirm`). When its
//! answer is lost, the runtime runs that read once, as a read, and reports
//! `landed`, `not_found` or `unknown` next to `action_outcome_unknown`. It
//! never resends the change; see `check_outcome`.
pub mod audit;
mod pagination;
mod preflight;
use itda_net::{Network, Request};
use itda_policy::{
    Approvals, Cursors, Effect, ExecutionRequest, ReviewInput, operation_effect, overrides_method,
    rendered_request_effect, safe_read_url, validate_spec,
};
use itda_spec::{Operation, Spec, interpolate_template_str};
use itda_types::{
    error::{AppError, AppResult, usage},
    json::{resolve_all, resolve_pointer},
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
    cursors: Option<Arc<Cursors>>,
}

/// One action call. `account` is only needed for actions with credentials.
#[derive(Clone, Debug, Default)]
pub struct Call {
    pub action: String,
    pub args: BTreeMap<String, String>,
    pub account: Option<String>,
    /// The id of an approval the user granted for this exact change.
    pub approval: Option<String>,
    /// The `meta.next` handle of the previous answer, to read the page
    /// after it with the same inputs.
    pub next: Option<String>,
    /// A read `itda setup` tries once while the person watches: a block or
    /// challenge is reported without pausing the site (see
    /// `itda_net::Request::pause_on_block`).
    pub discovery: bool,
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
    /// A handle for the next page, when the read turns pages and the answer
    /// said there is one. Pass it back as `next` with the same inputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// What the confirming read said about a change whose answer was lost.
#[derive(Clone, Debug, Serialize)]
pub struct OutcomeCheck {
    /// The read that was run.
    pub read: String,
    /// `landed`, `not_found` or `unknown`.
    pub result: &'static str,
    /// Why the read failed, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Runtime {
    pub fn new(net: Arc<Network>, vault: Arc<Vault>) -> Self {
        Self {
            net,
            vault,
            approvals: None,
            audit: None,
            cursors: None,
        }
    }

    /// Keep page markers in this store, so reads that turn pages can hand
    /// out `meta.next` and continue from it.
    pub fn with_pagination(mut self, cursors: Arc<Cursors>) -> Self {
        self.cursors = Some(cursors);
        self
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
        let mut reclassified = None;
        let mut result = self.run(spec, &call, &mut reclassified).await;
        let mut check = None;
        if let Err(error) = &mut result
            && error.public_code == "action_outcome_unknown"
            && let Ok(op) = spec.op(&call.action)
            && let Some(confirm) = &op.confirm
        {
            let outcome = self.check_outcome(spec, op, confirm, &call).await;
            describe_outcome(error, &outcome);
            check = Some(outcome);
        }
        if let Some(log) = &self.audit {
            let mut event = self.event(spec, &call, "call");
            if let Some(reason) = reclassified {
                event.effect = Effect::Write.as_str().into();
                event.reclassified = reason.into();
            }
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
            if let Some(check) = &check {
                event.outcome_check = check.result.into();
                event.confirm_read = check.read.clone();
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

    /// Run the read `confirm` names once, with its defaults and the change's
    /// account, and look for the change's input in its answer. This is a
    /// read like any other: it waits for its turn at the gate and pauses the
    /// site on a block. Only the first page is read. Nothing is resent.
    async fn check_outcome(
        &self,
        spec: &Spec,
        op: &Operation,
        confirm: &itda_spec::Confirm,
        call: &Call,
    ) -> OutcomeCheck {
        let read = Call {
            action: confirm.read.clone(),
            account: call.account.clone(),
            ..Call::default()
        };
        let outcome = |result: &'static str, error: Option<String>| OutcomeCheck {
            read: confirm.read.clone(),
            result,
            error,
        };
        // The read is run like any read: one whose rendered request asks
        // for a change goes the way of a change, and so is never sent here.
        let mut reclassified = None;
        let envelope = match self.run(spec, &read, &mut reclassified).await {
            Ok(envelope) => envelope,
            Err(error) => return outcome("unknown", Some(error.public_code)),
        };
        let expected = confirm.matching.input().and_then(|(_, name)| {
            call.args.get(name).cloned().or_else(|| {
                op.input(name)
                    .and_then(|param| param.default.as_ref())
                    .map(util::plain)
            })
        });
        let Some(expected) = expected else {
            return outcome("unknown", Some("input_missing".into()));
        };
        let found = resolve_all(&envelope.data, &confirm.matching.pointer)
            .into_iter()
            .any(|value| util::plain(value).trim() == expected.trim());
        outcome(if found { "landed" } else { "not_found" }, None)
    }

    /// `reclassified` is set when a declared read was treated as a change
    /// because of what the rendered request said, so the caller's audit
    /// record says the same.
    async fn run(
        &self,
        spec: &Spec,
        call: &Call,
        reclassified: &mut Option<&'static str>,
    ) -> AppResult<Envelope> {
        validate_spec(spec)?;
        let op = spec.op(&call.action)?;
        check_inputs(op, &call.args)?;
        let effect = operation_effect(op, &call.args);
        if call.next.is_some() {
            if op.pagination.is_none() || effect != Effect::Read {
                return Err(usage(
                    format!("{} does not turn pages", op.name),
                    "Call it without next.",
                ));
            }
            if op
                .pagination_input()
                .is_some_and(|input| call.args.contains_key(input))
            {
                return Err(usage(
                    "next and the page input were both given",
                    "Pass next alone to continue, or the page input alone to start there.",
                ));
            }
        }
        if let Some(query) = call.args.get("query")
            && itda_spec::graphql_too_deep(query)
        {
            return Err(usage(
                format!(
                    "the GraphQL document nests deeper than {} levels",
                    itda_spec::GRAPHQL_MAX_DEPTH
                ),
                "Send a flatter document.",
            ));
        }
        if !matches!(op.transport.as_str(), "" | "http") {
            return Err(usage(
                format!("{} needs a browser", op.name),
                "This version sends plain HTTP requests only.",
            ));
        }

        let (mut url, mut body) = spec.build_request(op, &call.args)?;
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
        // A session the site keeps in the query goes on the address of each
        // request just before it is sent, the way a cookie goes in a header;
        // the address the user reviews and the audit log never carry it.
        let mut query_session = None;
        if let Some(account) = &account
            && let Some(credential) = self.credential(spec, op, account, &destination).await?
        {
            if !credential.header.is_empty() {
                headers.push((credential.header, credential.value));
            }
            query_session = credential.query;
            exposed.extend(credential.redact);
            secrets.push(credential.secret);
        }
        let credential_hash = secrets.first().map(Secret::fingerprint).unwrap_or_default();

        // The marker behind `next` goes into the declared input of a copy of
        // the inputs: the agent's own inputs are what the audit log records.
        let mut args = call.args.clone();
        let turning = op
            .pagination
            .as_ref()
            .map(|pagination| -> AppResult<_> {
                let binding = pagination::binding(
                    spec,
                    op,
                    account.as_deref().unwrap_or_default(),
                    &call.args,
                    &credential_hash,
                )?;
                Ok((pagination, binding))
            })
            .transpose()?;
        if let Some(handle) = &call.next {
            let Some((pagination, binding)) = &turning else {
                return Err(usage(
                    format!("{} does not turn pages", op.name),
                    "Call it without next.",
                ));
            };
            let cursors = self.cursors.as_ref().ok_or_else(|| {
                usage(
                    "This itda keeps no page markers",
                    "Call the read without next.",
                )
            })?;
            let marker = cursors.resolve(handle, binding).await?;
            if !pagination::acceptable(pagination, &marker) {
                return Err(AppError::api("pagination_expired", 409));
            }
            if pagination.hides_input() {
                exposed.push(marker.clone());
            }
            args.insert(pagination.input().0.to_owned(), marker);
            (url, body) = spec.build_request(op, &args)?;
            if validate_url(&url)?.origin() != destination.origin() {
                return Err(AppError::api("unsafe_target", 400));
            }
        }

        // The request is read once more as it will be sent. A declared read
        // whose rendered address or body asks for a change is a change, and
        // goes to the user for approval with the reason; one whose effect
        // cannot be told is not sent. Nothing declared a change gets looser.
        let form = op.body.as_ref().is_some_and(|body| !body.form.is_empty());
        let effect = match effect {
            Effect::Read => {
                let rendered = rendered_request_effect(&validate_url(&url)?, body.as_deref(), form);
                match rendered.effect {
                    Effect::Read => Effect::Read,
                    Effect::Write => {
                        *reclassified = Some(rendered.reason);
                        Effect::Write
                    }
                    Effect::Unknown => {
                        let mut error = AppError::api("effect_unknown", 409);
                        error.msg = format!(
                            "{}.{} is declared a read, but itda cannot tell what the request it built does: {}",
                            spec.site.name, op.name, rendered.reason
                        );
                        error.remedy = format!(
                            "{}. Declare the action's effect as write, or give the read its GraphQL document, then review the site definition. itda sent nothing.",
                            capitalised(rendered.reason)
                        );
                        return Err(error);
                    }
                }
            }
            declared => declared,
        };
        let approvals = match (effect, &self.approvals) {
            (Effect::Read, _) => None,
            (_, Some(approvals)) => Some(approvals),
            (_, None) => return Err(AppError::api("writes_disabled", 403)),
        };

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
        request.pause_on_block = !call.discovery;

        if let Some(approvals) = approvals {
            let execution = ExecutionRequest {
                site_id: spec.site.name.clone(),
                action_id: op.name.clone(),
                account_id: request.account_id.clone(),
                inputs: call.args.clone(),
                spec_hash: util::hash(&serde_json::to_vec(spec)?),
                credential_hash: credential_hash.clone(),
            };
            match &call.approval {
                None => {
                    let mut inputs = review_inputs(op, &request.method, &url, &call.args);
                    if let Some(reason) = *reclassified {
                        inputs.push(ReviewInput {
                            label: "reclassified".into(),
                            value: reason.into(),
                        });
                    }
                    // The same change was sent before, its answer was lost,
                    // and the confirming read showed it landed: the person
                    // sees that before deciding to send it again.
                    let landed = self.audit.as_ref().and_then(|log| {
                        log.landed_before(
                            &spec.site.name,
                            &op.name,
                            &request.account_id,
                            &call.args,
                        )
                        .ok()
                        .flatten()
                    });
                    if let Some(previous) = &landed {
                        inputs.push(ReviewInput {
                            label: "note".into(),
                            value: format!(
                                "A change with these inputs was sent at {} and {} showed it landed. Approving sends it again.",
                                previous.at, previous.confirm_read
                            ),
                        });
                    }
                    let review = approvals
                        .request_approval(
                            execution,
                            title(&spec.site.title, &spec.site.name),
                            op.name.clone(),
                            request.account_id.clone(),
                            inputs,
                        )
                        .await?;
                    let mut error = AppError::api("approval_required", 409)
                        .with("approval", review.id.clone())
                        .with("expires_at", review.expires_at.clone());
                    error.msg = match *reclassified {
                        Some(reason) => {
                            error = error.with("reclassified", reason);
                            format!(
                                "{}.{} is declared a read, but the request built from its inputs changes the site ({reason}); it waits for the user's approval {}",
                                spec.site.name, op.name, review.id
                            )
                        }
                        None => format!(
                            "{}.{} changes the site and waits for the user's approval {}",
                            spec.site.name, op.name, review.id
                        ),
                    };
                    error.remedy = format!(
                        "Ask the user to run `itda approve {}`, then call again with approval {}.",
                        review.id, review.id
                    );
                    if let Some(previous) = landed {
                        error = error.with("previous_outcome", "landed");
                        error.msg.push_str(&format!(
                            "; the same change was sent at {} and {} showed it landed",
                            previous.at, previous.confirm_read
                        ));
                        error.remedy.push_str(
                            " Tell the user the earlier one landed, so they can decline this one.",
                        );
                    }
                    return Err(error);
                }
                Some(id) => approvals.consume(id, &execution).await?,
            }
            if let Some(log) = &self.audit {
                let mut event = self.event(spec, call, "change_started");
                event.outcome = "sending".into();
                if let Some(reason) = *reclassified {
                    event.effect = Effect::Write.as_str().into();
                    event.reclassified = reason.into();
                }
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
                &args,
                &request,
                &destination,
                query_session.as_ref(),
            )
            .await?;
            let preflight::Found {
                values: found,
                cookies,
            } = found;
            let mut args = args.clone();
            args.extend(found.clone());
            let (url, body) = spec.build_request(op, &args)?;
            if validate_url(&url)?.origin() != destination.origin() {
                return Err(AppError::api("unsafe_target", 400));
            }
            request.url = url;
            request.body = body;
            exposed.extend(
                cookies
                    .iter()
                    .filter(|(_, value)| value.len() >= 8)
                    .map(|(_, value)| value.clone()),
            );
            request.headers = preflight::with_cookies(request.headers, &cookies);
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
            // The request is classified again from what will actually be
            // sent: a value a step found must not turn a read into a change.
            if approvals.is_none() {
                let final_url = validate_url(&request.url)?;
                let mut effective = args.clone();
                for (name, param) in op
                    .params
                    .iter()
                    .chain(op.query.iter())
                    .chain(op.body.iter().flat_map(|body| body.inputs()))
                    .filter(|(_, param)| param.from_step())
                {
                    if let Some(template) = param.default.as_ref().and_then(Value::as_str) {
                        effective.insert(
                            name.clone(),
                            interpolate_template_str(template, &BTreeMap::new(), &args),
                        );
                    }
                }
                if operation_effect(op, &effective) != Effect::Read
                    || !safe_read_url(&final_url)
                    || rendered_request_effect(&final_url, request.body.as_deref(), form).effect
                        != Effect::Read
                {
                    let mut error = AppError::api("effect_changed", 409);
                    error.msg = format!(
                        "{}.{} is a read, but the request built after preflight would change the site",
                        spec.site.name, op.name
                    );
                    return Err(error);
                }
            }
        }
        if let Some((name, _)) = request
            .headers
            .iter()
            .find(|(name, _)| overrides_method(name))
        {
            return Err(usage(
                format!(
                    "{}.{} sets {name}, which overrides the method",
                    spec.site.name, op.name
                ),
                "itda sends the method it reviewed; remove the header from the definition.",
            ));
        }

        let read = approvals.is_none();
        let graphql = op.graphql_query(&call.args);
        if let Some((key, value)) = &query_session {
            request.url = itda_types::url::with_query_pair(&request.url, key, value);
        }
        let response = match self.net.fetch(request).await {
            Ok(response) => response,
            // A change that failed after it may have reached the site has an
            // unknown outcome. One that never left (waiting for a turn, name
            // lookup, connecting) was simply not sent and keeps its own code.
            Err(error)
                if !read
                    && itda_net::sent(&error)
                    && matches!(
                        error.public_code.as_str(),
                        "timeout" | "request_failed" | "too_large"
                    ) =>
            {
                return Err(AppError::api("action_outcome_unknown", 502));
            }
            Err(error) => return Err(error),
        };

        // A site that answers a change with a redirect has applied it and
        // points at the result (post, then redirect, then get); the redirect
        // is reported, never followed. A redirect to a sign-in page is the
        // site asking for one: the session is no longer accepted.
        let redirected = !read
            && response.defense == "none"
            && matches!(response.status, 301 | 302 | 303 | 307 | 308);
        let location = redirected
            .then(|| {
                validate_url(&response.url)
                    .ok()
                    .and_then(|from| from.join(response.header("location")).ok())
            })
            .flatten();
        if location
            .as_ref()
            .is_some_and(itda_types::url::looks_like_sign_in)
        {
            return Err(
                AppError::api("auth_required", 401).with("upstream_status", response.status)
            );
        }
        if !response.successful() && !redirected {
            return Err(refusal(&response, account.is_some()));
        }
        // A GraphQL answer can carry errors behind a 2xx. For a change that
        // means the site may or may not have applied it; for a read it is
        // the query that failed, not the site that changed.
        if graphql && graphql_errors(&response) {
            return Err(if read {
                AppError::api("query_failed", 502).with("upstream_status", response.status)
            } else {
                let mut error = AppError::api("action_outcome_unknown", 502)
                    .with("upstream_status", response.status);
                error.msg =
                    "the site answered the change with errors; it may or may not have applied it"
                        .into();
                error
            });
        }
        if read && !op.matches_response(&response) {
            return Err(AppError::api("response_mismatch", 502));
        }
        // The change has happened, so a reply that does not match the
        // contract is reported as missing data rather than a failure.
        let mut data = if redirected {
            serde_json::json!({
                "location": location.map(|to| to.to_string()).unwrap_or_default()
            })
        } else {
            match shape(spec, op, &response) {
                Ok(data) => data,
                Err(_) if !read => Value::Null,
                Err(error) => return Err(error),
            }
        };
        let mut next = None;
        if let (Some((pagination, binding)), Some(cursors)) = (&turning, &self.cursors)
            && read
            && let Some(marker) =
                pagination::next_marker(spec, op, pagination, &response, &data, &args)
        {
            if pagination.hides_input() {
                exposed.push(marker.clone());
            }
            next = Some(cursors.issue(binding, &marker).await?);
        }
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
                next,
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
                    query: None,
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
                // A session the site carries in the query: the secret is
                // the value itself, sent as `key=value` on every request.
                if let Some(key) = auth.value_from.strip_prefix("query:") {
                    let value = secret.expose().trim().to_owned();
                    if value.is_empty() {
                        return Err(AppError::api("auth_required", 401));
                    }
                    let redact = (value.len() >= 8)
                        .then(|| value.clone())
                        .into_iter()
                        .collect();
                    return Ok(Some(Credential {
                        header: String::new(),
                        value: String::new(),
                        redact,
                        secret,
                        query: Some((key.to_owned(), value)),
                    }));
                }
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
                    query: None,
                }))
            }
            _ => Err(usage(
                format!("{} uses a credential this version cannot send", op.name),
                "Supported kinds are header and browser.",
            )),
        }
    }
}

/// A credential ready to send, and the values to keep out of results: a
/// header (empty for a session that goes in the query) or a query pair.
struct Credential {
    header: String,
    value: String,
    redact: Vec<String>,
    secret: Secret,
    query: Option<(String, String)>,
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

/// What the user reviews: what the action does in the definition's words,
/// the exact request line and every input, long values cut with their length.
fn review_inputs(
    op: &Operation,
    method: &str,
    url: &str,
    args: &BTreeMap<String, String>,
) -> Vec<ReviewInput> {
    let mut inputs = Vec::new();
    if !op.desc.is_empty() {
        inputs.push(ReviewInput {
            label: "does".into(),
            value: op.desc.clone(),
        });
    }
    inputs.push(ReviewInput {
        label: "request".into(),
        value: format!("{method} {url}"),
    });
    inputs.extend(args.iter().map(|(label, value)| ReviewInput {
        label: label.clone(),
        value: shown(value),
    }));
    inputs
}

/// A reason as the start of a sentence.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// At most this many characters of one input value are shown in a review.
const REVIEW_VALUE_LIMIT: usize = 500;

/// A value as the review shows it: whole when short, else its start and
/// how long it is.
pub fn shown(value: &str) -> String {
    if value.chars().count() <= REVIEW_VALUE_LIMIT {
        return value.to_owned();
    }
    let start: String = value.chars().take(REVIEW_VALUE_LIMIT).collect();
    format!("{start}... ({} characters in all)", value.chars().count())
}

/// Whether a JSON answer carries a non-empty top-level `errors` list, as
/// GraphQL servers report failures.
fn graphql_errors(response: &itda_net::Response) -> bool {
    response
        .json()
        .as_ref()
        .and_then(|json| resolve_pointer(json, "/errors"))
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
}

/// Put what the confirming read said next to `action_outcome_unknown`: in
/// the message, the remedy and `outcome_check`. The read's answer itself is
/// not passed on, only whether the record was in it.
fn describe_outcome(error: &mut AppError, check: &OutcomeCheck) {
    let sent = if error.msg == error.public_code {
        "the change was sent and its answer was lost".to_owned()
    } else {
        error.msg.clone()
    };
    let read = &check.read;
    (error.msg, error.remedy) = match check.result {
        "landed" => (
            format!("{sent}; {read} shows the change landed"),
            "The change is in: tell the user it landed and do not ask for it again.".into(),
        ),
        "not_found" => (
            format!("{sent}; {read} does not show the change"),
            "The site shows no trace of the change. itda never resends on its own; if the user still wants it, call again and they review a new approval.".into(),
        ),
        _ => (
            format!(
                "{sent}; the confirming read {read} failed too ({})",
                check.error.as_deref().unwrap_or("unknown")
            ),
            error.remedy.clone(),
        ),
    };
    if let Ok(value) = serde_json::to_value(check) {
        error.extra.insert("outcome_check".into(), value);
    }
}

/// Every input must be declared and not filled by a preflight step or by the
/// previous page's answer; required ones and types are checked when the
/// request is built.
fn check_inputs(op: &Operation, args: &BTreeMap<String, String>) -> AppResult<()> {
    for name in args.keys() {
        if op.hidden_input(name) {
            return Err(usage(
                format!("{name} comes from the previous answer"),
                "Pass the meta.next handle as next instead.",
            ));
        }
        let declared = op.input(name).is_some_and(|param| !param.from_step());
        if !declared {
            let declared: Vec<&str> = op
                .params
                .keys()
                .chain(op.query.keys())
                .chain(
                    op.body
                        .iter()
                        .flat_map(|body| body.inputs().map(|(n, _)| n)),
                )
                .map(String::as_str)
                .collect();
            let remedy = match util::nearest(name, declared.iter().copied()) {
                Some(close) => {
                    format!("Did you mean {close}? Use only the inputs the action declares.")
                }
                None => "Use only the inputs the action declares.".into(),
            };
            return Err(usage(format!("Unknown input: {name}"), remedy));
        }
    }
    Ok(())
}

/// Why a response that was not a success failed. Blocks and expired sessions
/// have already paused the scope in the network layer. Every refusal carries
/// `upstream_status`; a 403 that is not a sign-in problem is `api_denied`,
/// with the site's own short reason when it gave one.
pub fn refusal(response: &itda_net::Response, signed_in: bool) -> AppError {
    let reason = provider_reason(response);
    let error = match (response.defense.as_str(), response.status) {
        ("none", 401) if signed_in => AppError::api("auth_required", 401),
        ("none", 403) if signed_in && reason.as_deref().is_some_and(session_rejected) => {
            AppError::api("auth_required", 401)
        }
        ("none", 403) => {
            let mut error = AppError::api("api_denied", 403);
            error.msg = match &reason {
                Some(reason) => format!("the site refused this request ({reason})"),
                None => "the site refused this request".into(),
            };
            if let Some(reason) = &reason {
                error = error.with("provider_reason", reason.clone());
            }
            error
        }
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
    };
    error.with("upstream_status", response.status)
}

/// The reasons a site may give for a refusal that itda passes on. Anything
/// else in the answer stays out of the error: a body can hold private data.
const PROVIDER_REASONS: [&str; 12] = [
    "access_denied",
    "permission_denied",
    "insufficient_scope",
    "insufficient_permissions",
    "service_disabled",
    "forbidden",
    "not_authorized",
    "unauthenticated",
    "invalid_token",
    "token_expired",
    "session_expired",
    "login_required",
];

/// A 403 whose reason says the session itself was rejected is a sign-in
/// problem, not a permission problem.
fn session_rejected(reason: &str) -> bool {
    matches!(
        reason,
        "unauthenticated"
            | "invalid_token"
            | "token_expired"
            | "session_expired"
            | "login_required"
    )
}

/// The site's own reason for a refusal, when its JSON answer names one of
/// [`PROVIDER_REASONS`] at a usual place. Free text is never taken.
fn provider_reason(response: &itda_net::Response) -> Option<String> {
    let json = response.json()?;
    [
        "/code",
        "/error/code",
        "/error/type",
        "/error/status",
        "/error",
        "/reason",
    ]
    .iter()
    .filter_map(|pointer| resolve_pointer(&json, pointer))
    .filter_map(Value::as_str)
    .map(|value| value.trim().to_ascii_lowercase().replace('-', "_"))
    .find(|value| PROVIDER_REASONS.contains(&value.as_str()))
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
