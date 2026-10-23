//! Bounded first-party requests. Redirects never carry a credential to another
//! origin, and private addresses are refused unless explicitly allowed.
//!
//! Derived from Hycli `src/net.rs` (Apache-2.0). Changes:
//! - [`Network::new`] is direct HTTP only, identifying itself as
//!   `itda/<version>`; a block or challenge pauses the site.
//! - [`Network::adaptive`] tries a read the site answered with a challenge once
//!   more in the installed browser, remembers per site, account and action
//!   which way worked, and never pauses a site for a challenge. A signed-in
//!   401 and a rate limit are handled the same either way.
//! - Pure URL checks moved to `itda_types::url`; pacing moved from the approval
//!   store into [`Gate`].
mod adaptive;
pub mod defend;
pub mod files;
pub mod gate;
mod strategy;

#[cfg(test)]
mod adaptive_tests;

pub use files::{Download, FileToSend, Multipart, SavedFile};
pub use gate::{Gate, Lease, Outcome, not_sent, sent};

use futures_util::StreamExt;
use itda_types::{
    error::{AppError, AppResult},
    url::validate_url,
    util,
};
use reqwest::{
    Method,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde_json::Value;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

pub const USER_AGENT: &str = concat!("itda/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseVerdict {
    Valid,
    Recover,
    Terminal,
}

pub struct Network {
    pub gate: Arc<Gate>,
    pub allow_local: bool,
    clients: std::sync::Mutex<std::collections::BTreeMap<String, reqwest::Client>>,
    browser: Option<Arc<adaptive::Browser>>,
    identity_cache: Option<std::path::PathBuf>,
}
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// Empty for anonymous requests. Pacing and pauses are per origin and account.
    pub account_id: String,
    pub credential_fingerprint: Option<String>,
    pub limit: usize,
    pub timeout: Duration,
    pub read_only: bool,
    pub follow_redirects: bool,
    /// The least time the site's definition wants between its requests. The
    /// gate's own pace is the floor; see [`gate::DEFAULT_INTERVAL`].
    pub interval: Option<Duration>,
    /// Whether a block or challenge pauses the site until the user resumes
    /// it. Only the reads `itda setup` tries once, with the person watching,
    /// turn this off: the answer is still classified and reported, and the
    /// caller sends nothing more to the site in that run.
    pub pause_on_block: bool,
    /// Save a successful answer to a file instead of reading it into
    /// memory; see [`files`].
    pub download: Option<Download>,
    /// Send a `multipart/form-data` body streamed from disk; `body` is then
    /// ignored.
    pub multipart: Option<Multipart>,
}
impl Request {
    /// A bounded anonymous GET that follows same-origin redirects.
    pub fn read(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            account_id: String::new(),
            credential_fingerprint: None,
            limit: 8 * 1024 * 1024,
            timeout: Duration::from_secs(45),
            read_only: true,
            follow_redirects: true,
            interval: None,
            pause_on_block: true,
            download: None,
            multipart: None,
        }
    }
}
pub struct Response {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// The body, or nothing when it was saved to `file`.
    pub bytes: Vec<u8>,
    pub duration_ms: i64,
    pub defense: String,
    /// Where a downloaded answer was saved, with its size and hash.
    pub file: Option<SavedFile>,
}
impl Response {
    /// Every line of a header that may come more than once, such as
    /// `Link`, joined with commas; empty when there is none.
    pub fn headers_joined(&self, key: &str) -> String {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
    pub fn cookie_value(&self, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        self.headers
            .iter()
            .rev()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .find_map(|(_, v)| {
                v.split(';')
                    .next()
                    .and_then(|pair| pair.trim().strip_prefix(&prefix))
                    .map(str::to_owned)
            })
    }
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.bytes).ok()
    }
    pub fn successful(&self) -> bool {
        (200..300).contains(&self.status) && self.defense == "none"
    }
}
/// Bodies and headers may hold private data, so `Debug` shows only their size.
impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("url", &self.url)
            .field("status", &self.status)
            .field("headers", &self.headers.len())
            .field("bytes", &self.bytes.len())
            .field("defense", &self.defense)
            .field("file", &self.file)
            .finish()
    }
}
impl itda_spec::ResponseLike for Response {
    fn url(&self) -> &str {
        &self.url
    }
    fn status(&self) -> u16 {
        self.status
    }
    fn header(&self, key: &str) -> &str {
        Response::header(self, key)
    }
    fn body(&self) -> &[u8] {
        &self.bytes
    }
    fn successful(&self) -> bool {
        Response::successful(self)
    }
}
impl Network {
    pub fn new(gate: Arc<Gate>, allow_local: bool) -> Self {
        Self {
            gate,
            allow_local,
            clients: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            browser: None,
            identity_cache: None,
        }
    }

    pub fn adaptive(gate: Arc<Gate>, allow_local: bool) -> Self {
        let mut network = Self::new(gate, allow_local);
        network.browser = Some(Arc::new(adaptive::Browser::Real(
            itda_browser::broker::BrowserClient::local(),
        )));
        network
    }

    pub fn with_browser(mut self, browser: itda_browser::broker::BrowserClient) -> Self {
        self.browser = Some(Arc::new(adaptive::Browser::Real(browser)));
        self
    }

    pub fn with_identity_cache(mut self, path: std::path::PathBuf) -> Self {
        self.identity_cache = Some(path);
        self
    }

    pub fn is_adaptive(&self) -> bool {
        self.browser.is_some()
    }

    /// Clear pauses for one account on an origin and for the origin as a whole.
    pub async fn resume(&self, origin: &str, account_id: &str) -> AppResult<()> {
        self.gate.resume(&format!("{origin}|{account_id}")).await?;
        self.gate.resume(&format!("{origin}|*")).await?;
        self.reset_browser(origin, account_id).await
    }

    pub async fn reset_browser(&self, origin: &str, account_id: &str) -> AppResult<()> {
        if let Some(browser) = &self.browser {
            let browser = browser.clone();
            let (origin, account) = (origin.to_owned(), account_id.to_owned());
            tokio::task::spawn_blocking(move || browser.reset(&origin, &account))
                .await
                .map_err(|_| AppError::api("internal", 500))??;
        }
        Ok(())
    }

    /// A client for the host. A buffered request gets a timeout for the
    /// whole exchange; a file transfer (`streaming`) gets one per read
    /// instead, so a long transfer is bounded by silence, not by size.
    fn pooled_client(
        &self,
        host: &str,
        ua: &str,
        addresses: &[SocketAddr],
        timeout: Duration,
        streaming: bool,
    ) -> AppResult<reqwest::Client> {
        let mut addr_keys: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        addr_keys.sort();
        let pool_key = format!(
            "{}|{}|{}|{}|{}",
            host,
            ua,
            timeout.as_secs(),
            if streaming { "stream" } else { "buffer" },
            addr_keys.join(",")
        );
        if let Ok(guard) = self.clients.lock()
            && let Some(existing) = guard.get(&pool_key)
        {
            return Ok(existing.clone());
        }
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());
        let builder = if streaming {
            builder.read_timeout(timeout.min(Duration::from_secs(90)))
        } else {
            builder.timeout(timeout.min(Duration::from_secs(90)))
        };
        let client = builder
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(6)
            .tcp_keepalive(Duration::from_secs(30))
            .user_agent(ua)
            .resolve_to_addrs(host, addresses)
            .build()
            .map_err(|_| AppError::api("internal", 500))?;
        if let Ok(mut guard) = self.clients.lock() {
            if guard.len() >= 64 {
                guard.clear();
            }
            guard.insert(pool_key, client.clone());
        }
        Ok(client)
    }

    pub async fn fetch(&self, input: Request) -> AppResult<Response> {
        self.fetch_validated(input, "", |_| ResponseVerdict::Valid)
            .await
    }

    pub async fn fetch_validated(
        &self,
        input: Request,
        contract: &str,
        mut inspect: impl FnMut(&Response) -> ResponseVerdict + Send,
    ) -> AppResult<Response> {
        // A file read streams to disk and is checked by its headers; it stays
        // on direct HTTP.
        if self.browser.is_some() && input.read_only && input.download.is_none() {
            return self.fetch_adaptive(input, contract, &mut inspect).await;
        }
        self.fetch_http(input, &mut None).await
    }

    async fn fetch_http(&self, input: Request, budget: &mut Option<usize>) -> AppResult<Response> {
        let initial = validate_url(&input.url)?;
        let user_agent = if self.is_adaptive() && input.read_only {
            let cache = self.identity_cache.clone();
            tokio::task::spawn_blocking(move || match cache {
                Some(path) => itda_browser::transport::user_agent_cached(&path),
                None => itda_browser::transport::user_agent(),
            })
            .await
            .map_err(|_| AppError::api("internal", 500))?
        } else {
            None
        };
        let mut target = initial.clone();
        let mut redirects = 0;
        let mut retries = 0;
        let start = Instant::now();
        loop {
            let addresses = resolve(&target, self.allow_local).await?;
            let origin = target.origin().ascii_serialization();
            let host_scope = format!("{origin}|*");
            let scope = format!("{origin}|{}", input.account_id);
            let lease = self.gate.acquire(&scope, input.interval).await?;
            // An adaptive read takes the site's turn as an admission the
            // browser transports share; everything else holds the site lease
            // until the answer is in.
            let host_lease = if self.is_adaptive() && input.read_only {
                self.gate.admit_origin(&origin, input.interval).await?;
                None
            } else {
                Some(self.gate.acquire(&host_scope, input.interval).await?)
            };
            let leases: Vec<&Lease<'_>> =
                std::iter::once(&lease).chain(host_lease.as_ref()).collect();
            let streaming = input.download.is_some() || input.multipart.is_some();
            let client = self.pooled_client(
                target.host_str().expect("validated host"),
                user_agent.as_deref().unwrap_or(USER_AGENT),
                &addresses,
                input.timeout,
                streaming,
            )?;
            let headers = default_headers(&target, &input, &origin)?;
            let method = Method::from_bytes(input.method.as_bytes())
                .map_err(|_| AppError::api("data_invalid", 400))?;
            let mut request = client.request(method, target.clone()).headers(headers);
            let mut upload = None;
            if let Some(parts) = &input.multipart {
                let body = files::multipart_body(parts).await.map_err(not_sent)?;
                request = request
                    .header(reqwest::header::CONTENT_TYPE, &body.content_type)
                    .header(reqwest::header::CONTENT_LENGTH, body.length)
                    .body(body.body);
                upload = Some((body.changed, body.complete));
            } else if let Some(body) = &input.body {
                request = request.body(body.clone());
            }
            // A long upload keeps its turn: both leases are renewed while
            // the body is being sent.
            let sending = request.send();
            let response = match renewing(sending, &leases).await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    let is_timeout = error.is_timeout();
                    drop(lease);
                    drop(host_lease);
                    if budget.is_none()
                        && input.read_only
                        && retries < 1
                        && (is_timeout || error.is_connect())
                    {
                        retries += 1;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    let mut failed = AppError::api(
                        if is_timeout {
                            "timeout"
                        } else {
                            "request_failed"
                        },
                        502,
                    );
                    if upload.as_ref().is_some_and(|(changed, _)| {
                        changed.load(std::sync::atomic::Ordering::SeqCst)
                    }) {
                        failed = failed.with("reason", "file_changed");
                    }
                    // A connection that never opened carried nothing.
                    return Err(if error.is_connect() {
                        not_sent(failed)
                    } else {
                        failed
                    });
                }
                Err(lost) => {
                    drop(lease);
                    drop(host_lease);
                    return Err(lost);
                }
            };
            let status = response.status().as_u16();
            let headers: Vec<(String, String)> = response
                .headers()
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|v| (key.as_str().to_owned(), v.to_owned()))
                })
                .collect();
            let retry_after = retry_after(&headers);
            let rate_limited = status == 429 || retry_after.is_some();
            let host_outcome = if rate_limited {
                Outcome::RateLimited(retry_after)
            } else {
                Outcome::Done
            };
            // A 2xx answer to a file read goes to disk when its headers say
            // it is the file; anything else is read into memory as usual,
            // so a refusal or a block is classified from what it says.
            let header = |name: &str| {
                headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.as_str())
                    .unwrap_or_default()
            };
            let content_length = response.content_length();
            let mut refused = None;
            if let Some(download) = &input.download
                && (200..300).contains(&status)
            {
                refused = Some(files::refusal(
                    download,
                    status,
                    header("content-type"),
                    header("content-disposition"),
                    header("content-range"),
                    content_length,
                ));
            }
            let mut file = None;
            let mut peeked = false;
            let mut too_large = false;
            let mut bytes = Vec::new();
            let mut failed = None;
            match (&input.download, refused) {
                // Only a look: the first bytes, then the answer is dropped.
                (Some(download), Some(None)) if download.peek => {
                    peeked = true;
                    let mut stream = response.bytes_stream();
                    match stream.next().await {
                        Some(Ok(chunk)) => {
                            bytes.extend_from_slice(&chunk[..chunk.len().min(files::SNIFF)]);
                        }
                        Some(Err(_)) => failed = Some(AppError::api("partial_answer", 502)),
                        None if !download.allow_empty => {
                            failed = Some(AppError::api("empty_answer", 502));
                        }
                        None => {}
                    }
                }
                (Some(download), Some(None)) => {
                    let (content_type, disposition) = (
                        header("content-type").to_owned(),
                        header("content-disposition").to_owned(),
                    );
                    let stream = response.bytes_stream();
                    match files::receive(
                        download,
                        target.as_str(),
                        &content_type,
                        &disposition,
                        content_length,
                        stream,
                        &leases,
                    )
                    .await
                    {
                        Ok((saved, sniff)) => {
                            bytes = sniff;
                            file = Some(saved);
                        }
                        Err(error) => failed = Some(error),
                    }
                }
                _ => {
                    // An adaptive read shares one byte budget across its
                    // attempts, so a challenge page counts against it too.
                    let limit = budget.unwrap_or(input.limit);
                    too_large = content_length.is_some_and(|length| length > limit as u64);
                    if !too_large {
                        let mut stream = response.bytes_stream();
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk.map_err(|_| AppError::api("request_failed", 502))?;
                            if chunk.len() > limit.saturating_sub(bytes.len()) {
                                too_large = true;
                                break;
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                    }
                }
            }
            if too_large {
                lease.release(host_outcome).await?;
                if let Some(host) = host_lease {
                    host.release(host_outcome).await?;
                } else {
                    self.gate.observe_origin(&origin, host_outcome).await?;
                }
                return Err(AppError::api("too_large", 413));
            }
            if let Some(error) = failed {
                lease.release(host_outcome).await?;
                if let Some(host) = host_lease {
                    host.release(host_outcome).await?;
                } else {
                    self.gate.observe_origin(&origin, host_outcome).await?;
                }
                return Err(error);
            }
            if let Some(remaining) = budget {
                *remaining = remaining.saturating_sub(bytes.len());
            }
            let (defense, _) = defend::classify(&defend::Observation {
                status_code: status,
                headers: headers.clone(),
                body_snippet: String::from_utf8_lossy(&bytes[..bytes.len().min(16_384)])
                    .into_owned(),
            });
            let blocked = defense != defend::Class::None && defense != defend::Class::RateLimit;
            let pause = blocked && input.pause_on_block;
            let outcome = if pause {
                Outcome::Pause(gate::BLOCKED)
            } else if blocked {
                host_outcome
            } else if status == 401 && !input.account_id.is_empty() {
                // A signed-in request that is no longer accepted: stop using the
                // session until the user reconnects, instead of retrying it.
                Outcome::Pause(gate::AUTH_REQUIRED)
            } else {
                host_outcome
            };
            lease.release(outcome).await?;
            // A site that refuses automated access refuses it for every account.
            let host_outcome = if pause { outcome } else { host_outcome };
            if let Some(host) = host_lease {
                host.release(host_outcome).await?;
            } else {
                self.gate.observe_origin(&origin, host_outcome).await?;
            }
            if input.follow_redirects
                && input.read_only
                && matches!(status, 301 | 302 | 303 | 307 | 308)
            {
                let location = headers
                    .iter()
                    .find(|(key, _)| key == "location")
                    .map(|(_, value)| value);
                // A redirect itda will not follow (another origin, an address
                // it refuses, such as one with a credential-named query, or
                // too many) is answered as the redirect it is, not an error.
                if let Some(location) = location
                    && let Ok(next) = target.join(location)
                    && validate_url(next.as_str()).is_ok()
                    && next.origin() == initial.origin()
                    && itda_policy::safe_read_url(&next)
                    && redirects < 4
                {
                    target = next;
                    redirects += 1;
                    continue;
                }
            }
            // A site that asks for a short wait (Retry-After of ten seconds or
            // less) gets it, once, for a read: the cooldown just recorded is
            // waited out by the gate before the request goes again. A longer
            // wait is reported instead. A change is never sent twice.
            if budget.is_none()
                && input.read_only
                && retries < 1
                && matches!(status, 429 | 503)
                && !blocked
                && retry_after.is_some_and(|wait| wait <= gate::SHORT_COOLDOWN)
            {
                retries += 1;
                continue;
            }
            // Automatic retries are bounded, read-only and never bypass a site's cooldown.
            if budget.is_none()
                && input.read_only
                && status == 503
                && defense == defend::Class::None
                && retries < 1
                && retry_after.is_none()
            {
                retries += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            // The site answered as if it had the file but did not send one
            // (a page, a part, a type the read does not take): the read
            // fails by that name. A block or refusal above says more.
            if let Some(Some(why)) = refused
                && defense == defend::Class::None
            {
                return Err(AppError::api(why, 502).with("upstream_status", status));
            }
            // A change answered before its file was all sent: the site has
            // the answer, itda does not know what it did with the file.
            if let Some((_, complete)) = &upload
                && !complete.load(std::sync::atomic::Ordering::SeqCst)
                && (200..300).contains(&status)
            {
                return Err(
                    AppError::api("request_failed", 502).with("reason", "upload_incomplete")
                );
            }
            if file.is_some() || peeked {
                bytes = Vec::new();
            }
            return Ok(Response {
                url: target.to_string(),
                status,
                headers,
                bytes,
                duration_ms: start.elapsed().as_millis() as i64,
                defense: defense.as_str().into(),
                file,
            });
        }
    }
}

/// Drive `sending` to its end, renewing both leases every
/// [`files::RENEW_EVERY`] so a long upload keeps the site's turn. A lost
/// turn ends the wait with `request_lease_lost`.
async fn renewing<T>(sending: impl Future<Output = T>, leases: &[&Lease<'_>]) -> AppResult<T> {
    let mut ticks = tokio::time::interval(files::RENEW_EVERY);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    tokio::pin!(sending);
    loop {
        tokio::select! {
            done = &mut sending => return Ok(done),
            _ = ticks.tick() => {
                for lease in leases {
                    lease.renew().await?;
                }
            }
        }
    }
}

/// Request headers shared by direct HTTP and browser reads.
fn default_headers(target: &url::Url, input: &Request, origin: &str) -> AppResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    let path_lower = target.path().to_ascii_lowercase();
    let is_mutation = !matches!(input.method.as_str(), "GET" | "HEAD" | "OPTIONS");
    let has_json_hint = input.body.is_some()
        || path_lower.contains("/api/")
        || path_lower.contains("graphql")
        || path_lower.ends_with(".json")
        || input.headers.iter().any(|(k, _)| {
            k.eq_ignore_ascii_case("content-type") || k.eq_ignore_ascii_case("authorization")
        });
    headers.insert(
        HeaderName::from_static("accept"),
        HeaderValue::from_static(if has_json_hint {
            "application/json, text/plain, */*"
        } else {
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        }),
    );
    // itda has no language preference of its own: an agent reads any language,
    // and a stated language changes which page some sites serve.
    headers.insert(
        HeaderName::from_static("accept-language"),
        HeaderValue::from_static("*"),
    );
    // Same-origin Referer/Origin let first-party CSRF checks pass for the user's own session.
    if (has_json_hint || is_mutation || !input.account_id.is_empty())
        && let Ok(value) = HeaderValue::from_str(&format!("{origin}/"))
    {
        headers.insert(HeaderName::from_static("referer"), value);
    }
    if is_mutation && let Ok(value) = HeaderValue::from_str(origin) {
        headers.insert(HeaderName::from_static("origin"), value);
    }
    for (key, value) in &input.headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| AppError::api("data_invalid", 400))?;
        let mut value =
            HeaderValue::from_str(value).map_err(|_| AppError::api("data_invalid", 400))?;
        if util::secret_field(key) {
            value.set_sensitive(true);
        }
        headers.insert(name, value);
    }
    Ok(headers)
}

pub async fn resolve(url: &url::Url, allow_local: bool) -> AppResult<Vec<SocketAddr>> {
    let host = url
        .host_str()
        .ok_or_else(|| AppError::api("bad_url", 400))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| AppError::api("bad_url", 400))?;
    let addresses: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(8),
        tokio::net::lookup_host((host.trim_matches(['[', ']']), port)),
    )
    .await
    .map_err(|_| not_sent(AppError::api("timeout", 502)))?
    .map_err(|_| not_sent(AppError::api("request_failed", 502)))?
    .collect();
    if addresses.is_empty() || (!allow_local && addresses.iter().any(|a| !public_ip(a.ip()))) {
        return Err(AppError::api("unsafe_target", 400));
    }
    Ok(addresses)
}
fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || s[0] & 0xe000 != 0x2000
                || (s[0] == 0x2001 && (s[1] == 0xdb8 || s[1] == 0 || s[1] == 2))
                || s[0] == 0x2002)
        }
    }
}
/// The wait a `Retry-After` header asks for. `0`, a date already past or a
/// value that is not a time is no wait at all: a site that sends
/// `Retry-After: 0` with every page is not rate limiting it.
pub fn retry_after(headers: &[(String, String)]) -> Option<Duration> {
    let value = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("retry-after"))
        .map(|(_, value)| value)?;
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return (seconds > 0).then(|| Duration::from_secs(seconds.min(86_400)));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let wait = date.duration_since(std::time::SystemTime::now()).ok()?;
    (!wait.is_zero()).then(|| wait.min(Duration::from_secs(86_400)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retry_after_of_zero_or_in_the_past_is_no_wait() {
        let header = |value: &str| vec![("retry-after".to_owned(), value.to_owned())];
        assert_eq!(retry_after(&header("0")), None);
        assert_eq!(retry_after(&header("3")), Some(Duration::from_secs(3)));
        assert_eq!(retry_after(&header("Wed, 21 Oct 2015 07:28:00 GMT")), None);
        assert_eq!(retry_after(&header("soon")), None);
        assert_eq!(retry_after(&[]), None);
    }

    #[test]
    fn a_read_asks_for_any_language_and_sends_no_browser_headers() {
        let names = |input: &Request| {
            let target = url::Url::parse(&input.url).unwrap();
            let origin = target.origin().ascii_serialization();
            let headers = default_headers(&target, input, &origin).unwrap();
            let mut names: Vec<String> = headers.keys().map(|k| k.as_str().to_owned()).collect();
            names.sort();
            (names, headers)
        };
        let (page, headers) = names(&Request::read("https://example.org/project/requests/"));
        assert_eq!(page, ["accept", "accept-language"]);
        assert!(headers["accept"].to_str().unwrap().starts_with("text/html"));
        assert_eq!(headers["accept-language"], "*");
        let (api, headers) = names(&Request::read("https://example.org/api/items"));
        assert_eq!(api, ["accept", "accept-language", "referer"]);
        assert_eq!(headers["accept"], "application/json, text/plain, */*");
        assert_eq!(headers["accept-language"], "*");
        let mut asked = Request::read("https://example.org/page");
        asked.headers = vec![("Accept-Language".into(), "ko".into())];
        let (with_language, headers) = names(&asked);
        assert_eq!(with_language, ["accept", "accept-language"]);
        assert_eq!(headers["accept-language"], "ko");
    }
}
