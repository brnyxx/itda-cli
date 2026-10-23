use crate::find;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{OnceLock, mpsc},
    time::{Duration, Instant},
};

static USER_AGENT: OnceLock<Option<String>> = OnceLock::new();

pub fn user_agent_cached(state_dir: &Path) -> Option<String> {
    cached_for(&find().ok()?, state_dir)
}

fn cached_for(browser: &Path, state_dir: &Path) -> Option<String> {
    let browser = browser.canonicalize().ok()?;
    let metadata = browser.metadata().ok()?;
    let identity = itda_types::util::hash(
        format!(
            "{}:{}:{:?}",
            browser.display(),
            metadata.len(),
            metadata.modified().ok()?
        )
        .as_bytes(),
    );
    let file = state_dir.join(format!("browser-identity-{identity}.json"));
    if let Ok(bytes) = itda_types::util::read_bounded(&file, 1024)
        && let Ok(agent) = serde_json::from_slice::<String>(&bytes)
        && agent.len() < 512
        && agent.starts_with("Mozilla/5.0 ")
        && !agent.chars().any(char::is_control)
    {
        return Some(agent);
    }
    let mut command = Command::new(browser);
    command.arg("--version");
    let agent = from_version(&probe(command, Duration::from_secs(2))?)?;
    if let Ok(bytes) = serde_json::to_vec(&agent) {
        let _ = itda_types::util::atomic_file(&file, &bytes);
    }
    Some(agent)
}

pub fn user_agent() -> Option<String> {
    USER_AGENT
        .get_or_init(|| {
            let mut command = Command::new(find().ok()?);
            command.arg("--version");
            let output = probe(command, Duration::from_secs(2))?;
            from_version(&output)
        })
        .clone()
}

fn probe(mut command: Command, timeout: Duration) -> Option<String> {
    let deadline = Instant::now().checked_add(timeout)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(4096).read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });
    let result = (|| {
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            match child.try_wait().ok()? {
                Some(status) => {
                    if !status.success() {
                        return None;
                    }
                    let bytes = receiver.recv_timeout(left).ok()?.ok()?;
                    return String::from_utf8(bytes).ok();
                }
                None => std::thread::sleep(left.min(Duration::from_millis(10))),
            }
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// The platform client hints name for this system, as Chrome reports it.
pub(super) fn hint_platform() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("macOS")
    } else if cfg!(target_os = "windows") {
        Some("Windows")
    } else if cfg!(target_os = "linux") {
        Some("Linux")
    } else {
        None
    }
}

fn from_version(output: &str) -> Option<String> {
    let version = output.split_whitespace().find_map(|word| {
        let parts = word.split('.').collect::<Vec<_>>();
        if parts.len() != 4 || parts.iter().any(|part| part.parse::<u32>().is_err()) {
            return None;
        }
        parts
            .first()?
            .parse::<u32>()
            .ok()
            .filter(|major| *major >= 40)
    })?;
    let platform = if cfg!(target_os = "macos") {
        "Macintosh; Intel Mac OS X 10_15_7"
    } else if cfg!(target_os = "windows") {
        "Windows NT 10.0; Win64; x64"
    } else if cfg!(target_os = "linux") {
        "X11; Linux x86_64"
    } else {
        return None;
    };
    Some(format!(
        "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version}.0.0.0 Safari/537.36"
    ))
}

#[cfg(test)]
mod tests {
    use super::{from_version, probe};
    use std::{process::Command, time::Duration};

    #[cfg(unix)]
    #[test]
    fn disk_identity_cache_skips_probe_and_tracks_executable_revision() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("itda-identity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let browser = dir.join("browser");
        let count = dir.join("probes");
        let script = |version| {
            format!(
                "#!/bin/sh\nprintf x >> '{}'\nprintf 'Chromium {version}\\n'\n",
                count.display()
            )
        };
        std::fs::write(&browser, script("199.0.0.0")).unwrap();
        std::fs::set_permissions(&browser, std::fs::Permissions::from_mode(0o700)).unwrap();
        let first = super::cached_for(&browser, &dir).unwrap();
        assert_eq!(super::cached_for(&browser, &dir), Some(first.clone()));
        assert_eq!(std::fs::read(&count).unwrap(), b"x");
        std::fs::write(&browser, script("200.0.12345.0")).unwrap();
        let second = super::cached_for(&browser, &dir).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&count).unwrap(), b"xx");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn identity_uses_the_probed_major_version() {
        assert!(
            from_version("Google Chrome 199.0.1234.5")
                .is_some_and(|s| s.contains("Chrome/199.0.0.0"))
        );
        assert!(from_version("not a browser version").is_none());
    }

    #[test]
    fn missing_browser_returns_no_identity() {
        assert!(
            probe(
                Command::new("/itda/missing-browser"),
                Duration::from_secs(1)
            )
            .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn hung_version_process_is_killed_at_its_deadline() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "while :; do :; done"]);
        assert!(probe(command, Duration::from_millis(30)).is_none());
    }
}
