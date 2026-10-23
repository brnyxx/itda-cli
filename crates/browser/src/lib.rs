//! The user's own browser: sign-in, recording, and reads that need one.
//!
//! For sign-in itda opens the user's own Chrome, Edge or Chromium in a fresh,
//! empty profile at the site's sign-in page, lets the person sign in by hand,
//! and then reads the cookies that sign-in left for that one site through the
//! DevTools protocol, and, for a site that keeps its session in the page's own
//! storage instead, that origin's `localStorage` and `sessionStorage`
//! ([`storage`]). The profile is deleted afterwards, so nothing else from the
//! user's browsing is ever visible, and cookies and storage of other sites are
//! never read.
//!
//! [`record`] opens the same kind of window and watches the requests the
//! person's own use of the site makes, so a definition can be drafted without
//! the browser's developer tools.
//!
//! [`transport`] runs a read inside a hidden browser window when direct HTTP
//! was answered with a challenge: one isolated session per site and account,
//! with every request it makes checked the way a direct one is. [`broker`]
//! keeps a few of those sessions warm across CLI calls in a worker process that
//! exits when idle; [`supervisor`] closes a browser whose worker died.
pub mod broker;
mod orphans;
pub mod record;
pub mod storage;
pub mod supervisor;
pub mod transport;

pub use record::{Capture, Recorder};
pub use storage::{StorageItem, StorageSession, bearer_session, looks_like_token};

use itda_types::error::{AppError, AppResult, usage};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tungstenite::{Message, WebSocket};

/// The largest DevTools message itda accepts: a 16 MiB answer as base64,
/// with its JSON around it.
const MESSAGE_LIMIT: usize = 24 << 20;

/// One cookie as the browser reports it.
#[derive(Clone, Debug, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    /// Seconds since the epoch; negative for a session cookie.
    #[serde(default)]
    pub expires: f64,
}

/// A browser window open at a sign-in page, in a profile of its own.
pub struct SignIn {
    child: Child,
    profile: PathBuf,
    socket: WebSocket<TcpStream>,
    next_id: u64,
}

impl SignIn {
    /// Open `browser` at `url` in a new temporary profile.
    pub fn open(browser: &Path, url: &url::Url) -> AppResult<Self> {
        let (child, profile, socket) = launch(browser, url, Duration::from_secs(15))?;
        Ok(Self {
            child,
            profile,
            socket,
            next_id: 0,
        })
    }

    /// Unexpired cookies the browser would send to `site`'s host.
    pub fn cookies(&mut self, site: &url::Url) -> AppResult<Vec<Cookie>> {
        let result = self.send(None, "Storage.getCookies", json!({}))?;
        site_cookies(&result, site)
    }

    /// The `localStorage` and `sessionStorage` items of `site`'s origin, from
    /// the page open there: where a single-page app keeps a session it has no
    /// cookie for. Read once, over the protocol; empty when no page is open at
    /// that origin.
    pub fn storage(&mut self, site: &url::Url) -> AppResult<Vec<StorageItem>> {
        storage::read(
            &mut |session, method, params| self.send(session, method, params),
            site,
            None,
        )
    }

    /// True while the person has not closed the browser.
    pub fn is_open(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Send one command to the browser, or to an attached page session, and
    /// wait for its reply; events arriving meanwhile are dropped.
    fn send(&mut self, session: Option<&str>, method: &str, params: Value) -> AppResult<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let lost = |_| AppError::api("browser_unavailable", 502);
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = session.into();
        }
        self.socket
            .send(Message::text(message.to_string()))
            .map_err(lost)?;
        loop {
            let message = self.socket.read().map_err(lost)?;
            let Ok(text) = message.to_text() else {
                continue;
            };
            let Ok(reply) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if reply["id"].as_u64() == Some(id) {
                return match reply.get("result") {
                    Some(result) => Ok(result.clone()),
                    None => Err(AppError::api("browser_unavailable", 502)),
                };
            }
        }
    }
}

impl Drop for SignIn {
    fn drop(&mut self) {
        let _ = self.send(None, "Browser.close", json!({}));
        close(&mut self.child, &self.profile);
    }
}

/// The unexpired cookies in a `Storage.getCookies` result that the browser
/// would send to `site`'s host.
pub(crate) fn site_cookies(result: &Value, site: &url::Url) -> AppResult<Vec<Cookie>> {
    let cookies: Vec<Cookie> = serde_json::from_value(result["cookies"].clone())
        .map_err(|_| AppError::api("browser_unavailable", 502))?;
    let host = site.host_str().unwrap_or_default().to_ascii_lowercase();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default();
    Ok(cookies
        .into_iter()
        .filter(|c| applies_to(&c.domain, &host))
        .filter(|c| c.expires <= 0.0 || c.expires > now)
        .collect())
}

/// Start the browser in a fresh profile at `url` and connect to its DevTools
/// socket. `read_timeout` bounds how long one read waits for a message.
pub(crate) fn launch(
    browser: &Path,
    url: &url::Url,
    read_timeout: Duration,
) -> AppResult<(Child, PathBuf, WebSocket<TcpStream>)> {
    launch_with(
        browser,
        url,
        read_timeout,
        &[],
        Instant::now() + Duration::from_secs(30),
    )
}

pub(crate) fn launch_with(
    browser: &Path,
    url: &url::Url,
    read_timeout: Duration,
    args: &[String],
    deadline: Instant,
) -> AppResult<(Child, PathBuf, WebSocket<TcpStream>)> {
    let registry = std::env::var_os("ITDA_BROWSER_REGISTRY").map(PathBuf::from);
    let profile = registry
        .as_ref()
        .map(|path| path.join("profiles"))
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("itda-browser-{}", uuid::Uuid::new_v4()));
    itda_types::util::private_dir(&profile)?;
    let supervisor = registry
        .as_ref()
        .and_then(|_| std::env::var_os("ITDA_BROWSER_SUPERVISOR"));
    let mut command = Command::new(supervisor.as_deref().unwrap_or(browser.as_os_str()));
    if supervisor.is_some() {
        command
            .arg("__browser-process")
            .arg("--browser")
            .arg(browser)
            .arg("--profile")
            .arg(&profile)
            .arg("--");
    }
    if registry.is_some() {
        command.arg("--enable-automation");
    }
    let mut child = command
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(args)
        .args([
            "--remote-debugging-port=0",
            "--remote-allow-origins=http://127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--new-window",
            url.as_str(),
        ])
        .stdin(if supervisor.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            // Nothing registered this profile yet; nobody else would remove it.
            let _ = orphans::remove_profile(&profile);
            usage(
                format!("Could not open {}", browser.display()),
                "Set ITDA_BROWSER to the path of Chrome, Edge or Chromium.",
            )
        })?;
    let connected = endpoint(&mut child, &profile, deadline).and_then(|(port, endpoint)| {
        let lost = |_| AppError::api("browser_unavailable", 502);
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(lost)?;
        stream
            .set_read_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))
            .map_err(lost)?;
        stream.set_write_timeout(Some(read_timeout)).map_err(lost)?;
        // The handshake needs whole reads; the bound applies to messages after it.
        // A 16 MiB answer is one base64 protocol message of about 22 MiB, over
        // the library's 16 MiB frame default.
        let config = tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MESSAGE_LIMIT))
            .max_frame_size(Some(MESSAGE_LIMIT));
        let (socket, _) =
            tungstenite::client::client_with_config(endpoint.as_str(), stream, Some(config))
                .map_err(|error| match error {
                    tungstenite::HandshakeError::Failure(tungstenite::Error::Io(error))
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        AppError::api("timeout", 502)
                    }
                    _ => AppError::api("browser_unavailable", 502),
                })?;
        socket
            .get_ref()
            .set_read_timeout(Some(read_timeout))
            .map_err(lost)?;
        if let Some(registry) = &registry {
            orphans::register(registry, &profile, &endpoint)?;
        }
        Ok(socket)
    });
    match connected {
        Ok(socket) => Ok((child, profile, socket)),
        Err(error) => {
            terminate(&mut child, &profile);
            Err(error)
        }
    }
}

/// The browser-level DevTools address the browser writes once it is ready.
fn endpoint(child: &mut Child, profile: &Path, deadline: Instant) -> AppResult<(u16, String)> {
    let file = profile.join("DevToolsActivePort");
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&file) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next())
                && let Ok(port) = port.parse::<u16>()
                && path.starts_with("/devtools/browser/")
            {
                return Ok((port, format!("ws://127.0.0.1:{port}{path}")));
            }
        }
        if !matches!(child.try_wait(), Ok(None)) {
            break;
        }
        std::thread::sleep(
            Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    Err(AppError::api(
        if Instant::now() >= deadline {
            "timeout"
        } else {
            "browser_unavailable"
        },
        502,
    ))
}

pub(crate) fn close(child: &mut Child, profile: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    terminate(child, profile);
}

pub(crate) fn terminate(child: &mut Child, profile: &Path) {
    if child.stdin.take().is_some() {
        let _ = child.wait();
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
    if orphans::remove_profile(profile).is_ok()
        && let Some(registry) = std::env::var_os("ITDA_BROWSER_REGISTRY")
    {
        orphans::forget(Path::new(&registry), profile);
    }
}

/// True when a cookie for `domain` is sent to `host` (RFC 6265 domain match).
pub fn applies_to(domain: &str, host: &str) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

/// Cookies as one `Cookie` header value, in the order given.
pub fn header(cookies: &[Cookie]) -> String {
    cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The browser to open: `ITDA_BROWSER`, or the first installed Chrome, Edge,
/// Chromium or Brave.
pub fn find() -> AppResult<PathBuf> {
    if let Some(path) = std::env::var_os("ITDA_BROWSER") {
        return Ok(PathBuf::from(path));
    }
    candidates()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            usage(
                "No Chrome, Edge or Chromium was found",
                "Install one, or set ITDA_BROWSER to its path.",
            )
        })
}

fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in [
            "Google Chrome",
            "Microsoft Edge",
            "Chromium",
            "Brave Browser",
        ] {
            out.push(format!("/Applications/{app}.app/Contents/MacOS/{app}").into());
        }
    } else if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
            if let Some(base) = std::env::var_os(base) {
                let base = PathBuf::from(base);
                out.push(base.join("Google/Chrome/Application/chrome.exe"));
                out.push(base.join("Microsoft/Edge/Application/msedge.exe"));
                out.push(base.join("Chromium/Application/chrome.exe"));
            }
        }
    } else {
        for name in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ] {
            for dir in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
                out.push(Path::new(dir).join(name));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_follow_the_domain_match_rule() {
        assert!(applies_to("example.com", "example.com"));
        assert!(applies_to(".example.com", "app.example.com"));
        assert!(!applies_to("app.example.com", "example.com"));
        assert!(!applies_to("ample.com", "example.com"));
        assert!(!applies_to("", "example.com"));
    }

    #[test]
    fn the_header_joins_name_value_pairs() {
        let cookie = |name: &str, value: &str| Cookie {
            name: name.into(),
            value: value.into(),
            domain: "example.com".into(),
            expires: -1.0,
        };
        assert_eq!(header(&[cookie("a", "1"), cookie("b", "2")]), "a=1; b=2");
    }
}
