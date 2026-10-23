use super::{
    Browser, BrowserMode, BrowserResponse, Call, cooldown,
    credentials::action_url,
    protocol::{field, header_pairs},
};
use itda_policy::{safe_graphql_read, safe_read_url};
use itda_types::error::{AppError, AppResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;

impl Browser {
    pub(super) fn intercept(
        &mut self,
        session: &str,
        params: &Value,
        call: &mut Call<'_>,
    ) -> AppResult<()> {
        let id = field(params, "requestId")?;
        let key = format!("{session}:{id}");
        let raw = params["request"]["url"].as_str().unwrap_or_default();
        let method = params["request"]["method"].as_str().unwrap_or_default();
        let response_stage = params.get("responseStatusCode").is_some()
            || params.get("responseErrorReason").is_some();
        if !call.settling
            && !response_stage
            && call.bootstrap.as_deref() == Some(raw)
            && method == "GET"
        {
            self.send(
                Some(session),
                "Fetch.fulfillRequest",
                json!({
                    "requestId":id, "responseCode":200,
                    "responseHeaders":[{"name":"Content-Type","value":"text/html"}],
                }),
                call.deadline,
            )?;
            return Ok(());
        }
        let target = action_url(raw);
        let same_origin = target
            .as_ref()
            .is_ok_and(|url| url.origin() == call.url.origin());
        let main_document = (session.is_empty() || session == call.session)
            && params["resourceType"] == "Document"
            && params["frameId"] == call.frame;
        let initial = !call.action_started
            && !response_stage
            && (session.is_empty() || session == call.session)
            && raw == call.url.as_str()
            && method == call.request.method
            && params.get("redirectedRequestId").is_none()
            && (call.request.mode == BrowserMode::Fetch || main_document);
        let redirected_action = params["redirectedRequestId"]
            .as_str()
            .is_some_and(|id| call.action_ids.contains(&format!("{session}:{id}")));
        let action = initial || redirected_action || call.action_ids.contains(&key);
        let graphql = method == "POST"
            && !response_stage
            && params.get("redirectedRequestId").is_none()
            && !call.read_posts.contains(&key)
            && target.as_ref().is_ok_and(|url| {
                params["request"]["postData"]
                    .as_str()
                    .is_some_and(|body| safe_graphql_read(url, body.as_bytes()))
            });
        let preflight = method == "OPTIONS"
            && header_pairs(&params["request"]["headers"])
                .iter()
                .any(|(name, value)| {
                    name == "access-control-request-method"
                        && matches!(value.as_str(), "GET" | "HEAD" | "POST")
                });
        let safe_url = target.as_ref().is_ok_and(|url| {
            (initial || (response_stage && action) || safe_read_url(url))
                && (!(action || main_document) || same_origin)
        });
        let safe_method = matches!(method, "GET" | "HEAD")
            || preflight
            || (method == "POST"
                && call.request.read_only
                && (initial
                    || graphql
                    || (response_stage
                        && (call.action_ids.contains(&key) || call.read_posts.contains(&key)))));
        if call.settling || !safe_url || !safe_method {
            self.send(
                Some(session),
                "Fetch.failRequest",
                json!({"requestId":id,"errorReason":"BlockedByClient"}),
                call.deadline,
            )?;
            if !call.settling && (main_document || action) {
                call.failure = Some(AppError::api("unsafe_target", 400));
            }
            return Ok(());
        }
        if initial {
            call.action_started = true;
        }
        if graphql {
            call.read_posts.insert(key.clone());
        }
        if action {
            call.action_ids.insert(key);
        }
        let origin = target?.origin().ascii_serialization();
        let now = cooldown::now_ms();
        self.cooldowns.retain(|_, until| *until > now);
        if !response_stage && let Some(&until) = self.cooldowns.get(&origin) {
            self.send(
                Some(session),
                "Fetch.failRequest",
                json!({"requestId":id,"errorReason":"BlockedByClient"}),
                call.deadline,
            )?;
            call.resource_limits.insert(origin, until);
            if action || main_document {
                let seconds = until.saturating_sub(now).saturating_add(999) / 1000;
                call.rate_limited = Some(BrowserResponse {
                    url: raw.to_owned(),
                    status: 429,
                    headers: vec![("retry-after".into(), seconds.to_string())],
                    bytes: Vec::new(),
                    resource_limits: Vec::new(),
                });
            }
            return Ok(());
        }
        if response_stage {
            let mut headers: Vec<(String, String)> = params["responseHeaders"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|header| {
                    Some((
                        header["name"].as_str()?.to_ascii_lowercase(),
                        header["value"].as_str()?.to_owned(),
                    ))
                })
                .collect();
            let status = params["responseStatusCode"]
                .as_u64()
                .and_then(|code| u16::try_from(code).ok())
                .unwrap_or(0);
            if let Some(until) = cooldown::retry_until(status, &headers, now) {
                self.send(
                    Some(session),
                    "Fetch.failRequest",
                    json!({"requestId":id,"errorReason":"BlockedByClient"}),
                    call.deadline,
                )?;
                let stored = self.cooldowns.entry(origin.clone()).or_insert(until);
                *stored = (*stored).max(until);
                call.resource_limits.insert(origin, *stored);
                if action || main_document {
                    if !same_origin {
                        headers.retain(|(name, _)| name != "set-cookie");
                    }
                    call.rate_limited = Some(BrowserResponse {
                        url: raw.to_owned(),
                        status,
                        headers,
                        bytes: Vec::new(),
                        resource_limits: Vec::new(),
                    });
                }
                return Ok(());
            }
            if params["resourceType"] == "Document" || !same_origin {
                let mut headers = params["responseHeaders"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if !same_origin {
                    headers.retain(|header| {
                        !header["name"]
                            .as_str()
                            .is_some_and(|name| name.eq_ignore_ascii_case("set-cookie"))
                    });
                }
                if params["resourceType"] == "Document" {
                    headers.push(json!({
                        "name":"Content-Security-Policy",
                        "value":"sandbox allow-scripts allow-same-origin; form-action 'none'; object-src 'none'; base-uri 'self'",
                    }));
                }
                self.send(Some(session), "Fetch.continueResponse", json!({
                    "requestId":id,"responseCode":params["responseStatusCode"],"responseHeaders":headers,
                }), call.deadline)?;
            } else {
                self.send(
                    Some(session),
                    "Fetch.continueRequest",
                    json!({"requestId":id}),
                    call.deadline,
                )?;
            }
            return Ok(());
        }
        let mut headers: BTreeMap<String, String> = header_pairs(&params["request"]["headers"])
            .into_iter()
            .collect();
        if same_origin {
            if initial {
                let session = call.session.clone();
                let cookies = self.command(
                    Some(&session),
                    "Network.getCookies",
                    json!({"urls":[raw]}),
                    call,
                )?;
                let cookie = cookies["cookies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|cookie| {
                        cookie["sourcePort"].as_i64().is_none_or(|port| {
                            port == -1
                                || Some(port) == call.url.port_or_known_default().map(i64::from)
                        })
                    })
                    .filter_map(|cookie| {
                        Some(format!(
                            "{}={}",
                            cookie["name"].as_str()?,
                            cookie["value"].as_str()?
                        ))
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                if cookies["cookies"]
                    .as_array()
                    .is_some_and(|cookies| !cookies.is_empty())
                {
                    headers.insert("cookie".into(), cookie);
                } else {
                    headers.remove("cookie");
                }
            }
            for (name, value) in &call.request.headers {
                let name = name.to_ascii_lowercase();
                if !matches!(
                    name.as_str(),
                    "cookie" | "user-agent" | "origin" | "referer"
                ) && !name.starts_with("sec-")
                {
                    headers.insert(name, value.clone());
                }
            }
        } else {
            headers.retain(|name, _| {
                matches!(
                    name.as_str(),
                    "accept"
                        | "content-type"
                        | "access-control-request-method"
                        | "access-control-request-headers"
                        | "accept-language"
                        | "accept-encoding"
                        | "user-agent"
                        | "origin"
                        | "range"
                        | "cache-control"
                        | "pragma"
                ) || name.starts_with("sec-fetch-")
                    || name.starts_with("sec-ch-")
            });
            headers.insert("cookie".into(), String::new());
            headers.insert("referer".into(), String::new());
        }
        self.send(Some(session), "Fetch.continueRequest", json!({
            "requestId":id,
            "headers":headers.into_iter().map(|(name,value)| json!({"name":name,"value":value})).collect::<Vec<_>>(),
        }), call.deadline)?;
        Ok(())
    }
}
