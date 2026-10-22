//! Bounded first-party requests. Redirects never carry a credential to another
//! origin, private addresses are refused unless explicitly allowed, and a site
//! that blocks or challenges a request is paused instead of retried.
//!
//! Derived from Hycli `src/net.rs` (Apache-2.0). Changes:
//! - The client identifies itself (`itda/<version>`) instead of presenting a
//!   Chrome user agent and `sec-ch-ua`/`sec-fetch-*` headers, and states no
//!   language preference (`Accept-Language: *`) instead of a Korean list.
//! - The in-page browser fetch and rendered-page fallbacks used after a block
//!   (`fetch_browser_in_page`, `fetch_rendered`) were removed, together with the
//!   strategy records that chose them.
//! - A block, challenge or signed-in 401 pauses the scope (see [`gate`]).
//! - Pure URL checks moved to `itda_types::url`; pacing moved from the approval
//!   store into [`Gate`].
pub mod defend;
pub mod gate;

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

pub struct Network {
    pub gate: Arc<Gate>,
    pub allow_local: bool,
    clients: std::sync::Mutex<std::collections::BTreeMap<String, reqwest::Client>>,
}
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// Empty for anonymous requests. Pacing and pauses are per origin and account.
    pub account_id: String,
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
            limit: 8 * 1024 * 1024,
            timeout: Duration::from_secs(45),
            read_only: true,
            follow_redirects: true,
            interval: None,
            pause_on_block: true,
        }
    }
}
pub struct Response {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub bytes: Vec<u8>,
    pub duration_ms: i64,
    pub defense: String,
}
impl Response {
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
        }
    }

    /// Clear pauses for one account on an origin and for the origin as a whole.
    pub async fn resume(&self, origin: &str, account_id: &str) -> AppResult<()> {
        self.gate.resume(&format!("{origin}|{account_id}")).await?;
        self.gate.resume(&format!("{origin}|*")).await
    }

    fn pooled_client(
        &self,
        host: &str,
        ua: &str,
        addresses: &[SocketAddr],
        timeout: Duration,
    ) -> AppResult<reqwest::Client> {
        let mut addr_keys: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        addr_keys.sort();
        let pool_key = format!(
            "{}|{}|{}|{}",
            host,
            ua,
            timeout.as_secs(),
            addr_keys.join(",")
        );
        if let Ok(guard) = self.clients.lock()
            && let Some(existing) = guard.get(&pool_key)
        {
            return Ok(existing.clone());
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(timeout.min(Duration::from_secs(90)))
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
        let initial = validate_url(&input.url)?;
        let mut target = initial.clone();
        let mut redirects = 0;
        let mut retries = 0;
        let start = Instant::now();
        loop {
            let addresses = resolve(&target, self.allow_local).await?;
            let origin = target.origin().ascii_serialization();
            // One request at a time per site, then per account on it. A short
            // cooldown the site asked for is waited out here as well.
            let host_scope = format!("{origin}|*");
            let host_lease = self.gate.acquire(&host_scope, input.interval).await?;
            let scope = format!("{origin}|{}", input.account_id);
            let lease = self.gate.acquire(&scope, input.interval).await?;
            let client = self.pooled_client(
                target.host_str().expect("validated host"),
                USER_AGENT,
                &addresses,
                input.timeout,
            )?;
            let headers = default_headers(&target, &input, &origin)?;
            let method = Method::from_bytes(input.method.as_bytes())
                .map_err(|_| AppError::api("data_invalid", 400))?;
            let mut request = client.request(method, target.clone()).headers(headers);
            if let Some(body) = &input.body {
                request = request.body(body.clone());
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    let is_timeout = error.is_timeout();
                    drop(lease);
                    drop(host_lease);
                    if input.read_only && retries < 1 && (is_timeout || error.is_connect()) {
                        retries += 1;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    let failed = AppError::api(
                        if is_timeout {
                            "timeout"
                        } else {
                            "request_failed"
                        },
                        502,
                    );
                    // A connection that never opened carried nothing.
                    return Err(if error.is_connect() {
                        not_sent(failed)
                    } else {
                        failed
                    });
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
            let mut too_large = response
                .content_length()
                .is_some_and(|length| length > input.limit as u64);
            let mut bytes = Vec::new();
            if !too_large {
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| AppError::api("request_failed", 502))?;
                    if bytes.len() + chunk.len() > input.limit {
                        too_large = true;
                        break;
                    }
                    bytes.extend_from_slice(&chunk);
                }
            }
            if too_large {
                lease.release(host_outcome).await?;
                host_lease.release(host_outcome).await?;
                return Err(AppError::api("too_large", 413));
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
            host_lease.release(host_outcome).await?;
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
            if input.read_only
                && retries < 1
                && matches!(status, 429 | 503)
                && !blocked
                && retry_after.is_some_and(|wait| wait <= gate::SHORT_COOLDOWN)
            {
                retries += 1;
                continue;
            }
            // Automatic retries are bounded, read-only and never bypass a site's cooldown.
            if input.read_only
                && status == 503
                && defense == defend::Class::None
                && retries < 1
                && retry_after.is_none()
            {
                retries += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            return Ok(Response {
                url: target.to_string(),
                status,
                headers,
                bytes,
                duration_ms: start.elapsed().as_millis() as i64,
                defense: defense.as_str().into(),
            });
        }
    }
}

/// Plain, honest request headers. The client never claims to be a browser.
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
