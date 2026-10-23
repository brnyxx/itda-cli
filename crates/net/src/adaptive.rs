use crate::{
    Network, Outcome, Request, Response, ResponseVerdict, default_headers, defend, gate, not_sent,
    resolve, retry_after,
    strategy::{StrategyKey, Transport},
};
use itda_browser::{
    broker::{BrowserClient, BrowserCompletion, BrowserFailure},
    transport::{BrowserMode, BrowserRequest, BrowserResponse},
};
use itda_types::{
    error::{AppError, AppResult},
    url::validate_url,
};
use std::time::Duration;
use tokio::time::{Instant, timeout_at};

/// The most an adaptive read may take across all of its attempts, whatever
/// the action's own timeout says.
const ATTEMPTS_CAP: Duration = Duration::from_secs(90);

pub(crate) enum Browser {
    Real(BrowserClient),
    #[cfg(test)]
    Test(std::sync::Mutex<Box<dyn FnMut(BrowserRequest) -> AppResult<BrowserResponse> + Send>>),
}

impl Browser {
    pub(crate) fn reset(&self, origin: &str, account: &str) -> AppResult<()> {
        match self {
            Self::Real(browser) => browser.reset(origin, account),
            #[cfg(test)]
            Self::Test(_) => Ok(()),
        }
    }

    fn execute(&self, input: BrowserRequest) -> Result<BrowserResponse, BrowserFailure> {
        match self {
            Self::Real(browser) => browser.execute(input),
            #[cfg(test)]
            Self::Test(execute) => execute.lock().unwrap_or_else(|p| p.into_inner())(input)
                .map_err(|error| BrowserFailure {
                    error,
                    completion: BrowserCompletion::Quiescent,
                }),
        }
    }
}

impl Network {
    pub(crate) async fn fetch_adaptive(
        &self,
        mut input: Request,
        contract: &str,
        inspect: &mut (impl FnMut(&Response) -> ResponseVerdict + Send),
    ) -> AppResult<Response> {
        let start = Instant::now();
        let deadline = start + input.timeout.min(ATTEMPTS_CAP);
        let target = validate_url(&input.url)?;
        let key = StrategyKey::new(&input, &target, contract)?;
        self.gate.clear_challenge_pause(&key).await?;
        let strategy = self.gate.strategy(&key).await?;
        let mut routes = std::collections::VecDeque::from(
            strategy.routes(input.method == "GET" && input.body.is_none()),
        );
        let mut broker_recovered = false;
        let mut budget = Some(input.limit);
        let mut last = None;
        let mut last_error = None;
        input.pause_on_block = false;
        while let Some(route) = routes.pop_front() {
            if Instant::now() >= deadline {
                return Err(not_sent(AppError::api("timeout", 502)));
            }
            let result = match route {
                Transport::Direct => {
                    let request = Request {
                        url: input.url.clone(),
                        method: input.method.clone(),
                        headers: input.headers.clone(),
                        body: input.body.clone(),
                        account_id: input.account_id.clone(),
                        credential_fingerprint: input.credential_fingerprint.clone(),
                        limit: input.limit,
                        timeout: deadline.saturating_duration_since(Instant::now()),
                        read_only: true,
                        follow_redirects: input.follow_redirects,
                        interval: input.interval,
                        download: None,
                        multipart: None,
                        pause_on_block: false,
                    };
                    timeout_at(deadline, self.fetch_http(request, &mut budget))
                        .await
                        .map_err(|_| AppError::api("timeout", 502))?
                        .map(|response| (response, Vec::new()))
                }
                Transport::Fetch | Transport::Render => {
                    self.fetch_browser(&input, route, budget.unwrap_or(0), deadline)
                        .await
                }
            };
            let (mut response, resource_limits) = match result {
                Ok(response) => response,
                Err(error)
                    if error
                        .extra
                        .get("completion")
                        .and_then(serde_json::Value::as_str)
                        == Some("unknown") =>
                {
                    return Err(error);
                }
                Err(error)
                    if !broker_recovered
                        && error
                            .extra
                            .get("broker_gone")
                            .and_then(serde_json::Value::as_bool)
                            == Some(true) =>
                {
                    broker_recovered = true;
                    routes.push_front(route);
                    continue;
                }
                // A browser that cannot start is skipped for a while; a
                // connection that failed says nothing about the route.
                Err(error) if error.public_code == "browser_unavailable" => {
                    self.gate.record_strategy(&key, route, false).await?;
                    last_error = Some(error);
                    continue;
                }
                Err(error) if error.public_code == "request_failed" => {
                    last_error = Some(error);
                    continue;
                }
                Err(error) if error.public_code == "response_too_large" => {
                    return Err(AppError::api("too_large", 413));
                }
                Err(error) => return Err(error),
            };
            if route != Transport::Direct
                && let Some(remaining) = &mut budget
            {
                *remaining = remaining.saturating_sub(response.bytes.len());
            }
            response.duration_ms = i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX);
            let verdict = if challenge(&response) {
                ResponseVerdict::Recover
            } else if response.successful() {
                inspect(&response)
            } else {
                ResponseVerdict::Terminal
            };
            match verdict {
                ResponseVerdict::Valid => {
                    self.gate.record_strategy(&key, route, true).await?;
                    return Ok(response);
                }
                ResponseVerdict::Terminal => return Ok(response),
                ResponseVerdict::Recover => {
                    if let Some(limit) = resource_limits
                        .iter()
                        .filter(|limit| limit.until_ms > self.gate.now())
                        .min_by_key(|limit| limit.until_ms)
                    {
                        let wait = limit
                            .until_ms
                            .saturating_sub(self.gate.now())
                            .max(0)
                            .saturating_add(999)
                            / 1000;
                        return Err(AppError::api("rate_limited", 429)
                            .with("retry_after", wait)
                            .with("upstream_origin", limit.origin.as_str())
                            .with("paused", false));
                    }
                    self.gate.record_strategy(&key, route, false).await?;
                }
            }
            last = Some(response);
        }
        // Cached failures are not observations about another request or account.
        // Returning one must never write a host pause or replay a cached body.
        last.ok_or_else(|| {
            last_error.unwrap_or_else(|| {
                let error = if contract.is_empty() {
                    AppError::api(gate::BLOCKED, 403)
                } else {
                    AppError::api("response_mismatch", 502)
                };
                not_sent(error.with("paused", false))
            })
        })
    }

    async fn fetch_browser(
        &self,
        input: &Request,
        route: Transport,
        limit: usize,
        deadline: Instant,
    ) -> AppResult<(Response, Vec<itda_browser::transport::OriginCooldown>)> {
        let target = validate_url(&input.url)?;
        timeout_at(deadline, resolve(&target, self.allow_local))
            .await
            .map_err(|_| not_sent(AppError::api("timeout", 502)))??;
        let origin = target.origin().ascii_serialization();
        let headers = default_headers(&target, input, &origin)?
            .iter()
            .map(|(key, value)| {
                value
                    .to_str()
                    .map(|value| (key.to_string(), value.to_owned()))
                    .map_err(|_| AppError::api("data_invalid", 400))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let browser = self
            .browser
            .as_ref()
            .ok_or_else(|| AppError::api("internal", 500))?
            .clone();
        let scope = format!("{origin}|{}", input.account_id);
        let request = BrowserRequest {
            url: input.url.clone(),
            method: input.method.clone(),
            headers,
            body: input.body.clone(),
            account_id: input.account_id.clone(),
            credential_fingerprint: input.credential_fingerprint.clone(),
            read_only: input.read_only,
            cooldowns: Vec::new(),
            limit,
            timeout: deadline.saturating_duration_since(Instant::now()),
            allow_local: self.allow_local,
            mode: match route {
                Transport::Fetch => BrowserMode::Fetch,
                Transport::Render => BrowserMode::Render,
                Transport::Direct => return Err(AppError::api("internal", 500)),
            },
        };
        let gate = self.gate.clone();
        let interval = input.interval;
        let signed_in = !input.account_id.is_empty();
        // The task owns the leases until blocking I/O finishes, even if its
        // caller is cancelled. The browser enforces the remaining deadline.
        let response = tokio::spawn(async move {
            let account = timeout_at(deadline, gate.acquire(&scope, interval))
                .await
                .map_err(|_| not_sent(AppError::api("timeout", 502)))??;
            timeout_at(deadline, gate.admit_origin(&origin, interval))
                .await
                .map_err(|_| not_sent(AppError::api("timeout", 502)))??;
            let cooldowns = gate.cooldowns().await?;
            let completed = tokio::task::spawn_blocking(move || {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(BrowserFailure {
                        error: not_sent(AppError::api("timeout", 502)),
                        completion: BrowserCompletion::NotStarted,
                    });
                }
                browser.execute(BrowserRequest {
                    timeout: remaining,
                    cooldowns,
                    ..request
                })
            })
            .await;
            let response = match completed {
                Ok(Ok(response)) => response,
                Ok(Err(failure)) => {
                    match failure.completion {
                        BrowserCompletion::Unknown => account.abandon(),
                        BrowserCompletion::NotStarted | BrowserCompletion::Quiescent => {
                            account.release(Outcome::Done).await?;
                        }
                    }
                    return Err(match failure.completion {
                        BrowserCompletion::Unknown => failure.error.with("completion", "unknown"),
                        BrowserCompletion::NotStarted => not_sent(failure.error),
                        BrowserCompletion::Quiescent => failure.error,
                    });
                }
                Err(_) => {
                    account.abandon();
                    return Err(AppError::api("request_failed", 502).with("completion", "unknown"));
                }
            };
            let host_outcome = if response.status == 429 || retry_after(&response.headers).is_some()
            {
                Outcome::RateLimited(retry_after(&response.headers))
            } else {
                Outcome::Done
            };
            let account_outcome = if response.status == 401 && signed_in {
                Outcome::Pause(gate::AUTH_REQUIRED)
            } else {
                host_outcome
            };
            account.release(account_outcome).await?;
            gate.observe_origin(&origin, host_outcome).await?;
            gate.observe_cooldowns(&response.resource_limits).await?;
            Ok::<_, AppError>(response)
        })
        .await
        .map_err(|_| AppError::api("internal", 500))??;
        let headers: Vec<_> = response
            .headers
            .into_iter()
            .map(|(key, value)| (key.to_ascii_lowercase(), value))
            .collect();
        if response.bytes.len() > limit {
            return Err(AppError::api("too_large", 413));
        }
        let final_url = validate_url(&response.url)?;
        if final_url.origin() != target.origin() {
            return Err(AppError::api("unsafe_target", 400));
        }
        let (defense, _) = defend::classify(&defend::Observation {
            status_code: response.status,
            headers: headers.clone(),
            body_snippet: String::from_utf8_lossy(
                &response.bytes[..response.bytes.len().min(16_384)],
            )
            .into_owned(),
        });
        Ok((
            Response {
                url: response.url,
                status: response.status,
                headers,
                bytes: response.bytes,
                duration_ms: 0,
                defense: defense.as_str().into(),
                file: None,
            },
            response.resource_limits,
        ))
    }
}

fn challenge(response: &Response) -> bool {
    if response.status == 401
        || response.defense == defend::Class::None.as_str()
        || response.defense == defend::Class::RateLimit.as_str()
    {
        return false;
    }
    let content_type = response.header("content-type").to_ascii_lowercase();
    let json = content_type.contains("application/json")
        || content_type.contains("+json")
        || response.json().is_some();
    !(response.status >= 400 && json)
}
