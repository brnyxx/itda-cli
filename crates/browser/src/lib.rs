//! Browser sign-in for sites without API keys.
//!
//! itda never drives a browser to use a site. It only opens the user's own
//! Chrome, Edge or Chromium in a fresh, empty profile at the site's sign-in
//! page, lets the person sign in by hand, and then reads the cookies that
//! sign-in left for that one site through the DevTools protocol. The profile
//! is deleted afterwards, so nothing else from the user's browsing is ever
//! visible, and cookies for other sites are dropped.
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
        let profile = std::env::temp_dir().join(format!("itda-sign-in-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&profile)?;
        let child = Command::new(browser)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--remote-debugging-port=0",
                "--remote-allow-origins=http://127.0.0.1",
                "--no-first-run",
                "--no-default-browser-check",
                "--new-window",
                url.as_str(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                usage(
                    format!("Could not open {}", browser.display()),
                    "Set ITDA_BROWSER to the path of Chrome, Edge or Chromium.",
                )
            })?;
        let mut child = child;
        let connected = endpoint(&mut child, &profile).and_then(|(port, endpoint)| {
            let lost = |_| AppError::api("browser_unavailable", 502);
            let stream = TcpStream::connect(("127.0.0.1", port)).map_err(lost)?;
            stream
                .set_read_timeout(Some(Duration::from_secs(15)))
                .map_err(lost)?;
            tungstenite::client(endpoint.as_str(), stream)
                .map(|(socket, _)| socket)
                .map_err(|_| AppError::api("browser_unavailable", 502))
        });
        match connected {
            Ok(socket) => Ok(Self {
                child,
                profile,
                socket,
                next_id: 0,
            }),
            Err(error) => {
                close(&mut child, &profile);
                Err(error)
            }
        }
    }

    /// Unexpired cookies the browser would send to `site`'s host.
    pub fn cookies(&mut self, site: &url::Url) -> AppResult<Vec<Cookie>> {
        let result = self.send("Storage.getCookies", json!({}))?;
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

    /// True while the person has not closed the browser.
    pub fn is_open(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn send(&mut self, method: &str, params: Value) -> AppResult<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let lost = |_| AppError::api("browser_unavailable", 502);
        self.socket
            .send(Message::text(
                json!({"id": id, "method": method, "params": params}).to_string(),
            ))
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
        let _ = self.send("Browser.close", json!({}));
        close(&mut self.child, &self.profile);
    }
}

/// The browser-level DevTools address the browser writes once it is ready.
fn endpoint(child: &mut Child, profile: &Path) -> AppResult<(u16, String)> {
    let file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + Duration::from_secs(30);
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
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(AppError::api("browser_unavailable", 502))
}

fn close(child: &mut Child, profile: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(profile);
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
