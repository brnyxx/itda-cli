use super::{BODY_LIMIT, Browser, BrowserResponse, Call};
use itda_types::error::{AppError, AppResult};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tungstenite::Message;

impl Browser {
    pub(super) fn send(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> AppResult<u64> {
        self.socket
            .get_ref()
            .set_write_timeout(Some(remaining(deadline)?))?;
        self.next_id += 1;
        let mut message = json!({"id":self.next_id,"method":method,"params":params});
        if let Some(session) = session.filter(|session| !session.is_empty()) {
            message["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::text(message.to_string()))
            .map_err(|_| unavailable())?;
        Ok(self.next_id)
    }

    pub(super) fn read(&mut self, deadline: Instant) -> AppResult<Value> {
        self.socket
            .get_ref()
            .set_read_timeout(Some(remaining(deadline)?))?;
        loop {
            let message = self.socket.read().map_err(|e| match e {
                tungstenite::Error::Io(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    AppError::api("timeout", 502)
                }
                _ => unavailable(),
            })?;
            if let Ok(text) = message.to_text() {
                return serde_json::from_str(text).map_err(|_| unavailable());
            }
            remaining(deadline)?;
        }
    }

    pub(super) fn command(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
        call: &mut Call<'_>,
    ) -> AppResult<Value> {
        let id = self.send(session, method, params, call.deadline)?;
        self.waiting.insert(id);
        let result = (|| {
            loop {
                if let Some(reply) = self.replies.remove(&id) {
                    return reply.get("result").cloned().ok_or_else(|| {
                        unavailable()
                            .with("detail", format!("{method}: {}", reply["error"]["code"]))
                    });
                }
                let message = self
                    .read(call.deadline)
                    .map_err(|error| error.with("detail", format!("waiting on {method}")))?;
                self.event(&message, call)?;
                if call.rate_limited.is_some() && !call.settling {
                    return Ok(json!({}));
                }
            }
        })();
        self.waiting.remove(&id);
        result
    }

    pub(super) fn event(&mut self, message: &Value, call: &mut Call<'_>) -> AppResult<()> {
        if let Some(id) = message["id"].as_u64() {
            if self.waiting.contains(&id) {
                self.replies.insert(id, message.clone());
            }
            return Ok(());
        }
        let params = &message["params"];
        let session = message["sessionId"].as_str().unwrap_or_default();
        if message["method"] == "Target.attachedToTarget" {
            let kind = params["targetInfo"]["type"].as_str().unwrap_or_default();
            if !call.sessions.contains(session)
                && !(session.is_empty() && matches!(kind, "shared_worker" | "service_worker"))
            {
                return Ok(());
            }
            let child = field(params, "sessionId")?;
            if call.sessions.len() >= 64 {
                return Err(AppError::api("response_too_large", 413));
            }
            if call.settling
                || !matches!(
                    kind,
                    "iframe" | "worker" | "shared_worker" | "service_worker"
                )
            {
                self.send(
                    None,
                    "Target.closeTarget",
                    json!({"targetId":params["targetInfo"]["targetId"]}),
                    call.deadline,
                )?;
                return Ok(());
            }
            call.sessions.insert(child.clone());
            if matches!(kind, "worker" | "shared_worker" | "service_worker") {
                let url = params["targetInfo"]["url"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                call.active
                    .retain(|_, (pending_url, _)| pending_url != &url);
                call.worker_scripts.insert(url);
            }
            if kind == "iframe" {
                let frame = field(&params["targetInfo"], "targetId")?;
                call.active
                    .retain(|_, (_, pending_frame)| pending_frame != &frame);
            }
            self.configure(&child, kind == "iframe", call)?;
            self.command(
                Some(&child),
                "Runtime.runIfWaitingForDebugger",
                json!({}),
                call,
            )?;
            return Ok(());
        }
        if message["method"] == "Target.detachedFromTarget" {
            if let Some(detached) = params["sessionId"].as_str() {
                call.sessions.remove(detached);
                call.active
                    .retain(|id, _| !id.starts_with(&format!("{detached}:")));
            }
            return Ok(());
        }
        if message["method"] == "Fetch.requestPaused" && session.is_empty() {
            return self.intercept(session, params, call);
        }
        if message["method"] == "Fetch.authRequired" && session.is_empty() {
            self.send(None, "Fetch.continueWithAuth", json!({
                "requestId":params["requestId"], "authChallengeResponse":{"response":"CancelAuth"},
            }), call.deadline)?;
            return Ok(());
        }
        if !call.sessions.contains(session) {
            return Ok(());
        }
        let key = format!(
            "{session}:{}",
            params["requestId"].as_str().unwrap_or_default()
        );
        match message["method"].as_str().unwrap_or_default() {
            "Fetch.requestPaused" => self.intercept(session, params, call)?,
            "Fetch.authRequired" => {
                self.send(Some(session), "Fetch.continueWithAuth", json!({
                    "requestId":params["requestId"], "authChallengeResponse":{"response":"CancelAuth"},
                }), call.deadline)?;
            }
            "Network.requestWillBeSent" => {
                let url = params["request"]["url"].as_str().unwrap_or_default();
                if call.action_network.is_none()
                    && session == call.session
                    && url == call.url.as_str()
                    && params["request"]["method"] == call.request.method
                {
                    call.action_network = Some(key.clone());
                }
                if call.action_network.as_ref() == Some(&key)
                    && params["redirectHasExtraInfo"] == true
                {
                    call.extra_expected += 1;
                }
                if !call.worker_scripts.contains(url) {
                    call.active.insert(
                        key,
                        (
                            url.to_owned(),
                            params["frameId"].as_str().unwrap_or_default().to_owned(),
                        ),
                    );
                }
            }
            "Network.responseReceivedExtraInfo" if call.action_network.as_ref() == Some(&key) => {
                call.extra_seen += 1;
                for (name, value) in header_pairs(&params["headers"]) {
                    if name == "set-cookie" {
                        for value in value.split('\n') {
                            call.response_cookies.push((name.clone(), value.to_owned()));
                        }
                    }
                }
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                call.active.remove(&key);
            }
            "Network.dataReceived" if !call.settling => {
                let length = params["dataLength"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(BODY_LIMIT);
                call.received = call.received.saturating_add(length);
                if call.received > call.request.limit.min(BODY_LIMIT).saturating_mul(4) {
                    return Err(AppError::api("response_too_large", 413));
                }
            }
            "Network.responseReceived" => {
                if call.action_network.as_ref() == Some(&key) && params["hasExtraInfo"] == true {
                    call.extra_expected += 1;
                }
                if !call.settling
                    && session == call.session
                    && params["type"] == "Document"
                    && params["frameId"] == call.frame
                    && call.bootstrap.is_none()
                {
                    let response = &params["response"];
                    call.response = Some(BrowserResponse {
                        url: field(response, "url")?,
                        status: response["status"]
                            .as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .ok_or_else(unavailable)?,
                        headers: header_pairs(&response["headers"]),
                        bytes: Vec::new(),
                        resource_limits: Vec::new(),
                    });
                }
            }
            "Page.loadEventFired" if session == call.session => {
                call.loaded = true;
                call.bootstrap = None;
            }
            _ => {}
        }
        Ok(())
    }
}

pub(super) fn field(value: &Value, key: &str) -> AppResult<String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(unavailable)
}

pub(super) fn header_pairs(value: &Value) -> Vec<(String, String)> {
    value
        .as_object()
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.to_ascii_lowercase(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn unavailable() -> AppError {
    AppError::api("browser_unavailable", 502)
}

pub(super) fn remaining(deadline: Instant) -> AppResult<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| AppError::api("timeout", 502))
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
