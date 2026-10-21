//! Record the requests a person's own use of a site makes, as a HAR document
//! in memory, so `itda draft har` can work from it without the browser's
//! developer tools and without the recording touching the disk.
//!
//! itda only watches. It attaches to every page the browser opens, turns on
//! the DevTools `Network` events, and keeps each request's method, address,
//! headers and body together with its response's status, headers and body.
//! It never clicks, types or navigates.
use crate::{close, launch};
use itda_types::error::{AppError, AppResult};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::TcpStream,
    path::{Path, PathBuf},
    process::Child,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket};

/// Longest request or response body kept, in bytes.
const BODY_LIMIT: usize = 1024 * 1024;
/// Most entries one recording keeps.
const ENTRY_LIMIT: usize = 2000;
/// Resource types a definition is drafted from; images, fonts, styles and
/// scripts say nothing about the site's calls.
const KEPT_TYPES: &[&str] = &[
    "Document",
    "XHR",
    "Fetch",
    "Other",
    "Prefetch",
    "EventSource",
];

/// What the capture wants fetched from the browser for one exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// `Network.getResponseBody`
    Body { session: String, request: String },
    /// `Network.getRequestPostData`
    PostData { session: String, request: String },
}

#[derive(Default)]
struct Exchange {
    seq: usize,
    started: String,
    url: String,
    method: String,
    headers: BTreeMap<String, String>,
    post_data: Option<String>,
    has_post: bool,
    kept: bool,
    response: Option<(u16, BTreeMap<String, String>, String)>,
    body: Option<String>,
    /// Replies still expected before the entry is complete.
    waiting: usize,
    /// `loadingFinished` arrived; the response may still be on its way.
    finished: bool,
}

/// Turns DevTools `Network` events into HAR entries. Pure: it only records
/// what it is told and says what else to ask for.
#[derive(Default)]
pub struct Capture {
    live: BTreeMap<String, Exchange>,
    /// Raw headers that arrived before their request.
    early_headers: BTreeMap<String, BTreeMap<String, String>>,
    entries: Vec<(usize, Value)>,
    seq: usize,
}

impl Capture {
    /// Feed one event. `session` tells pages apart; request ids repeat
    /// across them.
    pub fn event(&mut self, session: &str, method: &str, params: &Value) -> Vec<Fetch> {
        let Some(request_id) = params["requestId"].as_str() else {
            return Vec::new();
        };
        let key = format!("{session}:{request_id}");
        match method {
            "Network.requestWillBeSent" => {
                if let Some(redirect) = params.get("redirectResponse")
                    && let Some(mut previous) = self.live.remove(&key)
                {
                    previous.response = Some(response_parts(redirect));
                    self.finish(previous);
                }
                let request = &params["request"];
                let url = request["url"].as_str().unwrap_or_default();
                if !(url.starts_with("http://") || url.starts_with("https://"))
                    || url
                        .split(['?', '#'])
                        .next()
                        .is_some_and(|p| p.ends_with("/favicon.ico"))
                {
                    return Vec::new();
                }
                self.seq += 1;
                let mut headers = headers_map(&request["headers"]);
                if let Some(early) = self.early_headers.remove(&key) {
                    headers.extend(early);
                }
                let kept = params["type"]
                    .as_str()
                    .is_none_or(|t| KEPT_TYPES.contains(&t));
                self.live.insert(
                    key,
                    Exchange {
                        seq: self.seq,
                        started: wall_time(params["wallTime"].as_f64()),
                        url: url.to_owned(),
                        method: request["method"].as_str().unwrap_or("GET").to_owned(),
                        headers,
                        post_data: request["postData"].as_str().map(str::to_owned),
                        has_post: request["hasPostData"].as_bool().unwrap_or(false),
                        kept,
                        ..Exchange::default()
                    },
                );
            }
            "Network.requestWillBeSentExtraInfo" => {
                let raw = headers_map(&params["headers"]);
                match self.live.get_mut(&key) {
                    Some(exchange) => exchange.headers.extend(raw),
                    None => {
                        self.early_headers.insert(key, raw);
                    }
                }
            }
            "Network.responseReceived" => {
                // Linux builds have been seen to report a response for a
                // request they never announced; the response names the
                // address, so the exchange starts here, as a GET.
                if !self.live.contains_key(&key) {
                    let url = params["response"]["url"].as_str().unwrap_or_default();
                    if !recordable(url) {
                        return Vec::new();
                    }
                    self.seq += 1;
                    let headers = self.early_headers.remove(&key).unwrap_or_default();
                    self.live.insert(
                        key.clone(),
                        Exchange {
                            seq: self.seq,
                            started: wall_time(None),
                            url: url.to_owned(),
                            method: "GET".into(),
                            headers,
                            kept: true,
                            ..Exchange::default()
                        },
                    );
                }
                let Some(exchange) = self.live.get_mut(&key) else {
                    return Vec::new();
                };
                exchange.response = Some(response_parts(&params["response"]));
                if let Some(kind) = params["type"].as_str() {
                    exchange.kept &= KEPT_TYPES.contains(&kind);
                }
                // The end of the load was reported first; complete it now.
                if exchange.finished {
                    return self.complete(session, request_id, &key);
                }
            }
            "Network.loadingFinished" => {
                let Some(exchange) = self.live.get_mut(&key) else {
                    return Vec::new();
                };
                exchange.finished = true;
                if exchange.response.is_some() {
                    return self.complete(session, request_id, &key);
                }
            }
            "Network.loadingFailed" => {
                self.live.remove(&key);
            }
            _ => {}
        }
        Vec::new()
    }

    /// A loaded exchange: ask for its bodies, or record it when there are
    /// none to ask for. Events may arrive in either order, so this runs from
    /// whichever of `responseReceived` and `loadingFinished` comes last.
    fn complete(&mut self, session: &str, request_id: &str, key: &str) -> Vec<Fetch> {
        let Some(mut exchange) = self.live.remove(key) else {
            return Vec::new();
        };
        if !exchange.kept {
            return Vec::new();
        }
        let mut fetches = Vec::new();
        if exchange
            .response
            .as_ref()
            .is_some_and(|(_, _, mime)| textual(mime))
        {
            fetches.push(Fetch::Body {
                session: session.to_owned(),
                request: request_id.to_owned(),
            });
        }
        if exchange.has_post && exchange.post_data.is_none() {
            fetches.push(Fetch::PostData {
                session: session.to_owned(),
                request: request_id.to_owned(),
            });
        }
        exchange.waiting = fetches.len();
        if fetches.is_empty() {
            self.finish(exchange);
        } else {
            self.live.insert(key.to_owned(), exchange);
        }
        fetches
    }

    /// The reply to a [`Fetch`]; `None` when the browser had nothing.
    pub fn fetched(&mut self, fetch: &Fetch, reply: Option<&Value>) {
        let (session, request) = match fetch {
            Fetch::Body { session, request } | Fetch::PostData { session, request } => {
                (session, request)
            }
        };
        let key = format!("{session}:{request}");
        let Some(exchange) = self.live.get_mut(&key) else {
            return;
        };
        let text = reply.and_then(|reply| {
            let base64 = reply["base64Encoded"].as_bool().unwrap_or(false);
            let field = match fetch {
                Fetch::Body { .. } => "body",
                Fetch::PostData { .. } => "postData",
            };
            reply[field]
                .as_str()
                .filter(|text| !base64 && text.len() <= BODY_LIMIT)
                .map(str::to_owned)
        });
        match fetch {
            Fetch::Body { .. } => exchange.body = text,
            Fetch::PostData { .. } => exchange.post_data = text,
        }
        exchange.waiting = exchange.waiting.saturating_sub(1);
        if exchange.waiting == 0
            && let Some(exchange) = self.live.remove(&key)
        {
            self.finish(exchange);
        }
    }

    /// Entries recorded so far.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The recording as a HAR document, in request order. An exchange that
    /// got its response but never the end of its load, or whose body never
    /// came back, is kept without the body.
    pub fn har(mut self) -> Value {
        let late: Vec<Exchange> = std::mem::take(&mut self.live)
            .into_values()
            .filter(|exchange| exchange.response.is_some())
            .collect();
        for exchange in late {
            self.finish(exchange);
        }
        self.entries.sort_by_key(|(seq, _)| *seq);
        json!({"log": {
            "version": "1.2",
            "creator": {"name": "itda", "version": env!("CARGO_PKG_VERSION")},
            "entries": self.entries.into_iter().map(|(_, entry)| entry).collect::<Vec<_>>(),
        }})
    }

    fn finish(&mut self, exchange: Exchange) {
        if !exchange.kept || self.entries.len() >= ENTRY_LIMIT {
            return;
        }
        let Some((status, response_headers, mime)) = exchange.response else {
            return;
        };
        let mut request = Map::new();
        request.insert("method".into(), exchange.method.into());
        request.insert("url".into(), exchange.url.clone().into());
        request.insert("headers".into(), headers_list(&exchange.headers));
        request.insert(
            "queryString".into(),
            url::Url::parse(&exchange.url)
                .map(|url| {
                    url.query_pairs()
                        .map(|(name, value)| json!({"name": name, "value": value}))
                        .collect()
                })
                .unwrap_or_else(|_| Vec::new())
                .into(),
        );
        if let Some(text) = exchange.post_data {
            let mime = exchange
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            request.insert("postData".into(), json!({"mimeType": mime, "text": text}));
        }
        let mut content = Map::new();
        content.insert("mimeType".into(), mime.into());
        if let Some(body) = exchange.body {
            content.insert("size".into(), body.len().into());
            content.insert("text".into(), body.into());
        }
        self.entries.push((
            exchange.seq,
            json!({
                "startedDateTime": exchange.started,
                "request": request,
                "response": {
                    "status": status,
                    "headers": headers_list(&response_headers),
                    "content": content,
                },
            }),
        ));
    }
}

/// An address worth recording: a web address that is not the favicon.
fn recordable(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && !url
            .split(['?', '#'])
            .next()
            .is_some_and(|p| p.ends_with("/favicon.ico"))
}

fn response_parts(response: &Value) -> (u16, BTreeMap<String, String>, String) {
    (
        response["status"].as_u64().unwrap_or(0) as u16,
        headers_map(&response["headers"]),
        response["mimeType"].as_str().unwrap_or_default().to_owned(),
    )
}

fn headers_map(headers: &Value) -> BTreeMap<String, String> {
    headers
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned())))
        .collect()
}

fn headers_list(headers: &BTreeMap<String, String>) -> Value {
    headers
        .iter()
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect()
}

fn textual(mime: &str) -> bool {
    let mime = mime.to_ascii_lowercase();
    mime.contains("json")
        || mime.contains("html")
        || mime.contains("xml")
        || mime.starts_with("text/")
        || mime.contains("x-www-form-urlencoded")
}

fn wall_time(seconds: Option<f64>) -> String {
    let seconds = seconds.unwrap_or_default();
    let whole = seconds.trunc() as i64;
    let millis = ((seconds - seconds.trunc()) * 1000.0) as u32;
    let (days, secs) = (whole.div_euclid(86_400), whole.rem_euclid(86_400));
    // Civil-from-days, as in Howard Hinnant's date algorithms.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// A browser window being watched. Reads are bounded, so [`Recorder::poll`]
/// returns regularly for the caller to check whether to stop.
pub struct Recorder {
    child: Child,
    profile: PathBuf,
    socket: WebSocket<TcpStream>,
    next_id: u64,
    pending: BTreeMap<u64, Pending>,
    sessions: BTreeSet<String>,
    capture: Capture,
    /// The page to open once the first tab is being watched, so its own
    /// document request is recorded too.
    start: Option<url::Url>,
    /// The last events seen, for a test or a bug report: method, session
    /// and request id only, never a value.
    trace: std::collections::VecDeque<String>,
}

enum Pending {
    Fetch(Fetch),
    Ignore,
}

impl Recorder {
    /// Open `browser` at `url` in a new temporary profile and start watching
    /// every page it opens.
    pub fn open(browser: &Path, url: &url::Url) -> AppResult<Self> {
        // The window starts empty: a page that loads before the recorder is
        // attached would be missing its first requests.
        let blank = url::Url::parse("about:blank").map_err(|_| AppError::api("bad_url", 400))?;
        let (child, profile, socket) = launch(browser, &blank, Duration::from_millis(250))?;
        let mut recorder = Self {
            child,
            profile,
            socket,
            next_id: 0,
            pending: BTreeMap::new(),
            sessions: BTreeSet::new(),
            capture: Capture::default(),
            start: Some(url.clone()),
            trace: std::collections::VecDeque::new(),
        };
        let attached = recorder.send(
            None,
            "Target.setAutoAttach",
            json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true}),
        );
        if let Err(error) = attached {
            close(&mut recorder.child, &recorder.profile);
            return Err(error);
        }
        Ok(recorder)
    }

    /// Handle whatever the browser sends for up to `wait`.
    pub fn poll(&mut self, wait: Duration) -> AppResult<()> {
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            match self.socket.read() {
                Ok(message) => self.handle(&message)?,
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return Err(AppError::api("browser_unavailable", 502)),
            }
        }
        Ok(())
    }

    /// True while the person has not closed the browser.
    pub fn is_open(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Entries recorded so far.
    pub fn len(&self) -> usize {
        self.capture.len()
    }

    pub fn is_empty(&self) -> bool {
        self.capture.is_empty()
    }

    /// Unexpired cookies the browser would send to `site`'s host: the
    /// session the person signed in to while recording.
    pub fn cookies(&mut self, site: &url::Url) -> AppResult<Vec<crate::Cookie>> {
        let result = self.send(None, "Storage.getCookies", json!({}))?;
        crate::site_cookies(&result, site)
    }

    /// What the browser sent, newest last: one line per event or reply.
    pub fn trace(&self) -> Vec<String> {
        self.trace.iter().cloned().collect()
    }

    /// Close the window and return the recording.
    pub fn finish(mut self) -> Value {
        let _ = self.send(None, "Browser.close", json!({}));
        close(&mut self.child, &self.profile);
        std::mem::take(&mut self.capture).har()
    }

    fn handle(&mut self, message: &Message) -> AppResult<()> {
        let Ok(text) = message.to_text() else {
            return Ok(());
        };
        trace("<", text);
        let Ok(reply) = serde_json::from_str::<Value>(text) else {
            return Ok(());
        };
        let line = match reply["id"].as_u64() {
            Some(id) => format!(
                "reply {id} {}",
                if reply.get("error").is_some() {
                    "error"
                } else {
                    "ok"
                }
            ),
            None => format!(
                "{} {} {}",
                reply["method"].as_str().unwrap_or_default(),
                reply["sessionId"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(6)
                    .collect::<String>(),
                reply["params"]["requestId"]
                    .as_str()
                    .or_else(|| reply["params"]["targetInfo"]["type"].as_str())
                    .unwrap_or_default()
            ),
        };
        if self.trace.len() == 400 {
            self.trace.pop_front();
        }
        self.trace.push_back(line);
        if let Some(id) = reply["id"].as_u64() {
            match self.pending.remove(&id) {
                Some(Pending::Fetch(fetch)) => self.capture.fetched(&fetch, reply.get("result")),
                Some(Pending::Ignore) | None => {}
            }
            return Ok(());
        }
        let session = reply["sessionId"].as_str().unwrap_or_default().to_owned();
        match reply["method"].as_str().unwrap_or_default() {
            "Target.attachedToTarget" => {
                let session = reply["params"]["sessionId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                // Only pages make the site's calls; the browser's own UI,
                // extensions and workers are left alone. Sent without waiting:
                // a reply to an earlier command may still be on its way.
                let page = matches!(
                    reply["params"]["targetInfo"]["type"].as_str(),
                    Some("page" | "iframe" | "webview")
                );
                if page && !session.is_empty() && self.sessions.insert(session.clone()) {
                    self.send_async(Some(&session), "Network.enable", json!({}))?;
                    // Commands on one session run in order, so the page's
                    // first request is already being watched. Only the empty
                    // tab is sent there; extensions open pages of their own.
                    let blank =
                        reply["params"]["targetInfo"]["url"].as_str() == Some("about:blank");
                    if blank && let Some(start) = self.start.take() {
                        self.send_async(
                            Some(&session),
                            "Page.navigate",
                            json!({"url": start.as_str()}),
                        )?;
                    }
                }
            }
            "Target.detachedFromTarget" => {
                self.sessions
                    .remove(reply["params"]["sessionId"].as_str().unwrap_or_default());
            }
            method if method.starts_with("Network.") => {
                let fetches = self.capture.event(&session, method, &reply["params"]);
                for fetch in fetches {
                    let (cdp, request) = match &fetch {
                        Fetch::Body { request, .. } => ("Network.getResponseBody", request),
                        Fetch::PostData { request, .. } => ("Network.getRequestPostData", request),
                    };
                    let id = self.send_async(Some(&session), cdp, json!({"requestId": request}))?;
                    self.pending.insert(id, Pending::Fetch(fetch));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Send a command and keep handling events until its reply arrives.
    fn send(&mut self, session: Option<&str>, method: &str, params: Value) -> AppResult<Value> {
        let id = self.send_async(session, method, params)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let message = match self.socket.read() {
                Ok(message) => message,
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(_) => return Err(AppError::api("browser_unavailable", 502)),
            };
            let Ok(text) = message.to_text() else {
                continue;
            };
            let Ok(reply) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if reply["id"].as_u64() == Some(id) {
                trace("<", text);
                self.pending.remove(&id);
                return match reply.get("result") {
                    Some(result) => Ok(result.clone()),
                    None => Err(AppError::api("browser_unavailable", 502)),
                };
            }
            self.handle(&message)?;
        }
        Err(AppError::api("browser_unavailable", 502))
    }

    fn send_async(&mut self, session: Option<&str>, method: &str, params: Value) -> AppResult<u64> {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = session.into();
        }
        trace(">", &message.to_string());
        self.socket
            .send(Message::text(message.to_string()))
            .map_err(|_| AppError::api("browser_unavailable", 502))?;
        self.pending.insert(id, Pending::Ignore);
        Ok(id)
    }
}

/// `ITDA_DEBUG_CDP=1` prints every DevTools message to stderr, bodies cut
/// short, for debugging a recording that stays empty.
fn trace(direction: &str, text: &str) {
    if std::env::var_os("ITDA_DEBUG_CDP").is_some() {
        let cut: String = text.chars().take(300).collect();
        eprintln!("cdp{direction} {cut}");
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        close(&mut self.child, &self.profile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_one(capture: &mut Capture, session: &str, id: &str) -> Vec<Fetch> {
        let mut fetches = capture.event(
            session,
            "Network.requestWillBeSent",
            &json!({"requestId": id, "wallTime": 1_760_000_000.5, "type": "XHR", "request": {
                "url": "https://intranet.example/api/notices?q=x",
                "method": "POST",
                "headers": {"Content-Type": "application/json"},
                "postData": "{\"title\":\"hi\"}"
            }}),
        );
        fetches.extend(capture.event(
            session,
            "Network.requestWillBeSentExtraInfo",
            &json!({"requestId": id, "headers": {"cookie": "JSESSIONID=secret"}}),
        ));
        fetches.extend(capture.event(
            session,
            "Network.responseReceived",
            &json!({"requestId": id, "type": "XHR", "response": {
                "status": 201, "mimeType": "application/json", "headers": {"x-id": "7"}
            }}),
        ));
        fetches.extend(capture.event(
            session,
            "Network.loadingFinished",
            &json!({"requestId": id}),
        ));
        fetches
    }

    #[test]
    fn events_become_har_entries_with_bodies_fetched_on_request() {
        let mut capture = Capture::default();
        let fetches = capture_one(&mut capture, "s1", "1");
        assert_eq!(
            fetches,
            [Fetch::Body {
                session: "s1".into(),
                request: "1".into()
            }]
        );
        assert!(capture.is_empty());
        capture.fetched(
            &fetches[0],
            Some(&json!({"body": "{\"id\": 7}", "base64Encoded": false})),
        );
        assert_eq!(capture.len(), 1);
        let har = capture.har();
        let entry = &har["log"]["entries"][0];
        assert_eq!(entry["startedDateTime"], "2025-10-09T08:53:20.500Z");
        assert_eq!(entry["request"]["method"], "POST");
        assert_eq!(entry["request"]["postData"]["mimeType"], "application/json");
        assert_eq!(entry["request"]["queryString"][0]["name"], "q");
        let headers = entry["request"]["headers"].as_array().unwrap();
        assert!(headers.iter().any(|h| h["name"] == "cookie"));
        assert_eq!(entry["response"]["status"], 201);
        assert_eq!(entry["response"]["content"]["text"], "{\"id\": 7}");
    }

    #[test]
    fn ids_repeat_across_sessions_and_other_resources_are_dropped() {
        let mut capture = Capture::default();
        let first = capture_one(&mut capture, "s1", "1");
        let second = capture_one(&mut capture, "s2", "1");
        capture.fetched(
            &first[0],
            Some(&json!({"body": "a", "base64Encoded": false})),
        );
        capture.fetched(&second[0], None);
        assert_eq!(capture.len(), 2);

        capture.event(
            "s1",
            "Network.requestWillBeSent",
            &json!({"requestId": "img", "type": "Image", "request": {"url": "https://intranet.example/a.png", "method": "GET", "headers": {}}}),
        );
        capture.event(
            "s1",
            "Network.responseReceived",
            &json!({"requestId": "img", "type": "Image", "response": {"status": 200, "mimeType": "image/png", "headers": {}}}),
        );
        assert!(
            capture
                .event(
                    "s1",
                    "Network.loadingFinished",
                    &json!({"requestId": "img"})
                )
                .is_empty()
        );
        capture.event(
            "s1",
            "Network.requestWillBeSent",
            &json!({"requestId": "data", "request": {"url": "data:text/plain,hi", "method": "GET", "headers": {}}}),
        );
        assert_eq!(capture.len(), 2);
    }

    #[test]
    fn redirects_keep_both_hops_and_failures_are_dropped() {
        let mut capture = Capture::default();
        let start = json!({"requestId": "r", "type": "Document", "wallTime": 0.0, "request": {"url": "https://a.example/old", "method": "GET", "headers": {}}});
        capture.event("s", "Network.requestWillBeSent", &start);
        let hop = json!({"requestId": "r", "type": "Document", "wallTime": 1.0,
            "redirectResponse": {"status": 302, "mimeType": "", "headers": {"location": "/new"}},
            "request": {"url": "https://a.example/new", "method": "GET", "headers": {}}});
        capture.event("s", "Network.requestWillBeSent", &hop);
        capture.event("s", "Network.loadingFailed", &json!({"requestId": "r"}));
        let har = capture.har();
        let entries = har["log"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["response"]["status"], 302);
        assert_eq!(entries[0]["request"]["url"], "https://a.example/old");
    }

    #[test]
    fn events_in_either_order_and_bodies_that_never_come_still_record() {
        let mut capture = Capture::default();
        let start = |id: &str| json!({"requestId": id, "type": "Fetch", "wallTime": 1.0, "request": {"url": "https://a.example/api", "method": "GET", "headers": {}}});
        let answer = |id: &str| json!({"requestId": id, "type": "Fetch", "response": {"status": 200, "mimeType": "application/json", "headers": {}}});
        // The end of the load reported before the response.
        capture.event("s", "Network.requestWillBeSent", &start("1"));
        assert!(
            capture
                .event("s", "Network.loadingFinished", &json!({"requestId": "1"}))
                .is_empty()
        );
        let fetches = capture.event("s", "Network.responseReceived", &answer("1"));
        assert_eq!(fetches.len(), 1);
        capture.fetched(
            &fetches[0],
            Some(&json!({"body": "{}", "base64Encoded": false})),
        );
        assert_eq!(capture.len(), 1);
        // A body that is never delivered.
        capture.event("s", "Network.requestWillBeSent", &start("2"));
        capture.event("s", "Network.responseReceived", &answer("2"));
        assert_eq!(
            capture
                .event("s", "Network.loadingFinished", &json!({"requestId": "2"}))
                .len(),
            1
        );
        let har = capture.har();
        let entries = har["log"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[1]["response"]["content"].get("text").is_none());
    }

    #[test]
    fn a_response_without_its_request_and_a_load_that_never_ends_are_kept() {
        let mut capture = Capture::default();
        // Seen on Linux: the response arrives for a request never announced.
        let answer = json!({"requestId": "9", "type": "Fetch", "response": {"url": "https://a.example/api/items?q=x", "status": 200, "mimeType": "application/json", "headers": {}}});
        let fetches = capture.event("s", "Network.responseReceived", &answer);
        assert!(fetches.is_empty());
        let fetches = capture.event("s", "Network.loadingFinished", &json!({"requestId": "9"}));
        assert_eq!(fetches.len(), 1);
        capture.fetched(
            &fetches[0],
            Some(&json!({"body": "{}", "base64Encoded": false})),
        );
        // And a document whose load is never reported as finished.
        capture.event("s", "Network.requestWillBeSent", &json!({"requestId": "d", "type": "Document", "wallTime": 2.0, "request": {"url": "https://a.example/board", "method": "GET", "headers": {}}}));
        capture.event("s", "Network.responseReceived", &json!({"requestId": "d", "type": "Document", "response": {"url": "https://a.example/board", "status": 200, "mimeType": "text/html", "headers": {}}}));
        let har = capture.har();
        let urls: Vec<&str> = har["log"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["request"]["url"].as_str().unwrap())
            .collect();
        assert_eq!(
            urls,
            ["https://a.example/api/items?q=x", "https://a.example/board"]
        );
        assert_eq!(har["log"]["entries"][0]["request"]["method"], "GET");
    }

    #[test]
    fn wall_times_are_iso_dates() {
        assert_eq!(wall_time(Some(0.0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(wall_time(Some(951_782_400.25)), "2000-02-29T00:00:00.250Z");
    }
}
