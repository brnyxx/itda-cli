use super::encoding;
use super::protocol::{field, remaining, unavailable};
use super::{
    BODY_LIMIT, Browser, BrowserMode, BrowserRequest, BrowserResponse, Call, OriginCooldown,
    cooldown,
};
use itda_types::error::{AppError, AppResult};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use url::Url;

impl Browser {
    pub(super) fn execute(
        &mut self,
        request: &BrowserRequest,
        url: &Url,
        deadline: Instant,
    ) -> AppResult<BrowserResponse> {
        cooldown::seed(&mut self.cooldowns, &request.cooldowns, cooldown::now_ms())?;
        let mut call = Call {
            request,
            url,
            deadline,
            session: String::new(),
            frame: String::new(),
            sessions: BTreeSet::new(),
            worker_scripts: BTreeSet::new(),
            bootstrap: None,
            response: None,
            loaded: false,
            settling: true,
            action_started: false,
            action_ids: BTreeSet::new(),
            read_posts: BTreeSet::new(),
            action_network: None,
            extra_expected: 0,
            extra_seen: 0,
            active: BTreeMap::new(),
            received: 0,
            failure: None,
            rate_limited: None,
            response_cookies: Vec::new(),
            resource_limits: BTreeMap::new(),
        };
        self.prepare(&mut call)?;
        self.proxy.activate(request.allow_local, deadline)?;
        call.settling = false;
        call.loaded = false;
        let session = call.session.clone();
        let mut response = 'response: {
            match request.mode {
                BrowserMode::Fetch => {
                    let bootstrap = format!("{}/.itda-blank-{}", self.origin, uuid::Uuid::new_v4());
                    call.bootstrap = Some(bootstrap.clone());
                    self.command(
                        Some(&session),
                        "Page.navigate",
                        json!({"url":bootstrap}),
                        &mut call,
                    )?;
                    while !call.loaded {
                        let event = self.read(deadline)?;
                        self.event(&event, &mut call)?;
                    }
                    call.loaded = false;
                    call.active.clear();
                    call.received = 0;
                    let expression = format!(
                        r#"(async () => {{
                        try {{
                            const r = await fetch({}, {{
                                method: {}, body: {}, credentials: 'include',
                                redirect: 'follow', signal: AbortSignal.timeout({})
                            }});
                            const reader = r.body && r.body.getReader();
                            const parts = [];
                            let length = 0;
                            if (reader) {{
                                for (;;) {{
                                    const {{done, value}} = await reader.read();
                                    if (done) break;
                                    if (length + value.length > {}) {{
                                        await reader.cancel();
                                        return {{error:'response_too_large'}};
                                    }}
                                    parts.push(value);
                                    length += value.length;
                                }}
                            }}
                            const bytes = new Uint8Array(length);
                            let offset = 0;
                            for (const part of parts) {{ bytes.set(part, offset); offset += part.length; }}
                            let binary = '';
                            for (let i = 0; i < bytes.length; i += 0x8000) {{
                                binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
                            }}
                            return {{url:r.url, status:r.status, headers:Array.from(r.headers), b64:btoa(binary)}};
                        }} catch (e) {{
                            return {{error:e.name === 'TimeoutError' ? 'timeout' : 'request_failed'}};
                        }}
                    }})()"#,
                        json!(url.as_str()),
                        json!(request.method),
                        match &request.body {
                            Some(bytes) => format!(
                                "Uint8Array.from(atob({}), c => c.charCodeAt(0))",
                                json!(encoding::encode(bytes))
                            ),
                            None => "undefined".into(),
                        },
                        remaining(deadline)?.as_millis().max(1),
                        request.limit.min(BODY_LIMIT),
                    );
                    let result = self.command(
                        Some(&session),
                        "Runtime.evaluate",
                        json!({
                            "expression":expression, "awaitPromise":true, "returnByValue":true,
                        }),
                        &mut call,
                    )?;
                    if let Some(response) = call.rate_limited.take() {
                        break 'response response;
                    }
                    if let Some(error) = call.failure.take() {
                        return Err(error);
                    }
                    let value = &result["result"]["value"];
                    if let Some(code) = value["error"].as_str() {
                        return Err(AppError::api(
                            code,
                            if code == "response_too_large" {
                                413
                            } else {
                                502
                            },
                        ));
                    }
                    call.response = Some(BrowserResponse {
                        url: field(value, "url")?,
                        status: value["status"]
                            .as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .ok_or_else(unavailable)?,
                        headers: serde_json::from_value(value["headers"].clone())?,
                        bytes: encoding::decode(value["b64"].as_str().ok_or_else(unavailable)?)?,
                        resource_limits: Vec::new(),
                    });
                }
                BrowserMode::Render => {
                    let navigation = self.command(
                        Some(&session),
                        "Page.navigate",
                        json!({"url":url.as_str()}),
                        &mut call,
                    )?;
                    if let Some(response) = call.rate_limited.take() {
                        break 'response response;
                    }
                    if navigation.get("errorText").is_some() {
                        return Err(call
                            .failure
                            .take()
                            .unwrap_or_else(|| AppError::api("request_failed", 502)));
                    }
                    while !call.loaded || !call.active.is_empty() {
                        let event = self.read(deadline).map_err(|error| {
                            error.with(
                                "detail",
                                format!(
                                    "loaded={} pending_requests={}",
                                    call.loaded,
                                    call.active.len()
                                ),
                            )
                        })?;
                        self.event(&event, &mut call)?;
                        if let Some(response) = call.rate_limited.take() {
                            break 'response response;
                        }
                        if let Some(error) = call.failure.take() {
                            return Err(error);
                        }
                    }
                    let wait = remaining(deadline)?.saturating_sub(Duration::from_millis(150));
                    let expression = format!(
                        "({})({}, {})",
                        include_str!("render.js"),
                        request.limit.min(BODY_LIMIT),
                        wait.as_millis()
                    );
                    let result = self.command(
                        Some(&session),
                        "Runtime.evaluate",
                        json!({
                            "expression":expression, "returnByValue":true, "awaitPromise":true,
                        }),
                        &mut call,
                    )?;
                    if let Some(response) = call.rate_limited.take() {
                        break 'response response;
                    }
                    if let Some(error) = call.failure.take() {
                        return Err(error);
                    }
                    let value = &result["result"]["value"];
                    if value.get("error").is_some() {
                        return Err(AppError::api("response_too_large", 413));
                    }
                    let response = call
                        .response
                        .as_mut()
                        .ok_or_else(|| AppError::api("request_failed", 502))?;
                    response.bytes =
                        encoding::decode(value["b64"].as_str().ok_or_else(unavailable)?)?;
                }
            }
            call.response.take().ok_or_else(unavailable)?
        };
        while call.extra_seen < call.extra_expected {
            let event = self.read(deadline)?;
            self.event(&event, &mut call)?;
        }
        self.settle(&mut call)?;
        response.headers.append(&mut call.response_cookies);
        let now = cooldown::now_ms();
        self.cooldowns.retain(|_, until| *until > now);
        response.resource_limits = call
            .resource_limits
            .into_iter()
            .filter(|(_, until)| *until > now)
            .map(|(origin, until_ms)| OriginCooldown { origin, until_ms })
            .collect();
        Ok(response)
    }
}
