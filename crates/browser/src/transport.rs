use crate::{find, launch_with};
use itda_types::error::{AppError, AppResult};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    net::TcpStream,
    path::PathBuf,
    process::Child,
    time::{Duration, Instant},
};
use tungstenite::WebSocket;
use url::Url;

mod cooldown;
mod credentials;
mod encoding;
mod execute;
mod guard;
mod identity;
mod intercept;
mod protocol;
mod proxy;
mod session;

#[cfg(test)]
mod lifecycle_tests;

pub use identity::{user_agent, user_agent_cached};
use protocol::{remaining, unavailable};
use proxy::GuardedProxy;

const CACHE_LIMIT: usize = 4;
const BODY_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserMode {
    Fetch,
    Render,
}

#[derive(Clone, Debug)]
pub struct BrowserRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub account_id: String,
    pub credential_fingerprint: Option<String>,
    pub read_only: bool,
    pub limit: usize,
    pub timeout: Duration,
    pub allow_local: bool,
    pub mode: BrowserMode,
    pub cooldowns: Vec<OriginCooldown>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OriginCooldown {
    pub origin: String,
    pub until_ms: i64,
}

#[derive(Debug)]
pub struct BrowserResponse {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub bytes: Vec<u8>,
    pub resource_limits: Vec<OriginCooldown>,
}

#[derive(Default)]
pub struct BrowserTransport {
    browsers: VecDeque<Browser>,
}

impl BrowserTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self, origin: &str, account: &str) {
        self.browsers
            .retain(|b| b.origin != origin || b.account != account);
    }

    pub fn execute(&mut self, request: BrowserRequest) -> AppResult<BrowserResponse> {
        let deadline = Instant::now()
            .checked_add(request.timeout)
            .ok_or_else(|| AppError::api("data_invalid", 400))?;
        remaining(deadline)?;
        let url = credentials::action_url(&request.url)?;
        if !matches!(request.method.as_str(), "GET" | "HEAD" | "POST")
            || !request.read_only
            || (request.mode == BrowserMode::Render
                && (request.method != "GET" || request.body.is_some()))
        {
            return Err(AppError::api("unsafe_method", 400));
        }
        if request.limit == 0 {
            return Err(AppError::api("data_invalid", 400));
        }
        if request.body.as_ref().is_some_and(|b| b.len() > BODY_LIMIT) {
            return Err(AppError::api("response_too_large", 413));
        }
        for (name, value) in &request.headers {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c))
                || value.bytes().any(|c| c == 0 || c == b'\r' || c == b'\n')
                || matches!(
                    name.to_ascii_lowercase().as_str(),
                    "host"
                        | "connection"
                        | "content-length"
                        | "transfer-encoding"
                        | "upgrade"
                        | "proxy-authorization"
                        | "proxy-connection"
                )
            {
                return Err(AppError::api("data_invalid", 400));
            }
        }
        guard::resolve(&url, request.allow_local, deadline)?;
        let origin = url.origin().ascii_serialization();
        let cached = self
            .browsers
            .iter()
            .position(|b| b.origin == origin && b.account == request.account_id);
        let mut browser = match cached.and_then(|i| self.browsers.remove(i)) {
            Some(browser) => browser,
            None => {
                if self.browsers.len() == CACHE_LIMIT {
                    self.browsers.pop_front();
                }
                Browser::open(origin, request.account_id.clone(), deadline)?
            }
        };
        let result = browser.execute(&request, &url, deadline);
        let stopped = browser.proxy.deactivate();
        if result.is_ok() && stopped.is_ok() {
            self.browsers.push_back(browser);
        }
        stopped?;
        result
    }
}

struct Page {
    session: String,
    frame: String,
}

struct Browser {
    origin: String,
    account: String,
    child: Child,
    profile: PathBuf,
    socket: WebSocket<TcpStream>,
    proxy: GuardedProxy,
    next_id: u64,
    waiting: BTreeSet<u64>,
    replies: BTreeMap<u64, Value>,
    context: Option<String>,
    page: Option<Page>,
    credential_cookies: BTreeMap<String, String>,
    credential_digest: Option<String>,
    cooldowns: BTreeMap<String, i64>,
    /// The browser's own user agent without the `Headless` mark, asked of the
    /// browser once: what direct HTTP sends too.
    agent: Option<String>,
}

struct Call<'a> {
    request: &'a BrowserRequest,
    url: &'a Url,
    deadline: Instant,
    session: String,
    frame: String,
    sessions: BTreeSet<String>,
    worker_scripts: BTreeSet<String>,
    bootstrap: Option<String>,
    response: Option<BrowserResponse>,
    loaded: bool,
    settling: bool,
    action_started: bool,
    action_ids: BTreeSet<String>,
    read_posts: BTreeSet<String>,
    action_network: Option<String>,
    extra_expected: usize,
    extra_seen: usize,
    active: BTreeMap<String, (String, String)>,
    received: usize,
    failure: Option<AppError>,
    rate_limited: Option<BrowserResponse>,
    response_cookies: Vec<(String, String)>,
    resource_limits: BTreeMap<String, i64>,
}

impl Browser {
    fn open(origin: String, account: String, deadline: Instant) -> AppResult<Self> {
        let proxy = GuardedProxy::new()?;
        let blank = Url::parse("about:blank").map_err(|_| unavailable())?;
        let args = vec![
            "--headless=new".into(),
            "--disable-blink-features=AutomationControlled".into(),
            "--disable-background-networking".into(),
            "--disable-component-update".into(),
            "--disable-sync".into(),
            "--disable-quic".into(),
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
            format!("--proxy-server=socks5://{}", proxy.address()),
            "--proxy-bypass-list=<-loopback>".into(),
        ];
        let (child, profile, socket) = launch_with(
            &find().map_err(|_| unavailable())?,
            &blank,
            remaining(deadline)?,
            &args,
            deadline,
        )
        .map_err(|error| {
            if Instant::now() >= deadline || error.public_code == "timeout" {
                AppError::api("timeout", 502)
            } else {
                unavailable()
            }
        })?;
        Ok(Self {
            origin,
            account,
            child,
            profile,
            socket,
            proxy,
            next_id: 0,
            waiting: BTreeSet::new(),
            replies: BTreeMap::new(),
            context: None,
            page: None,
            credential_cookies: BTreeMap::new(),
            credential_digest: None,
            cooldowns: BTreeMap::new(),
            agent: None,
        })
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.proxy.deactivate();
        crate::terminate(&mut self.child, &self.profile);
    }
}
