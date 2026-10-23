use itda_types::{
    error::{AppError, AppResult},
    util,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    net::TcpStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tungstenite::Message;

#[derive(Serialize, Deserialize)]
struct Record {
    profile: PathBuf,
    endpoint: String,
    owner_pid: u32,
}

fn record_path(registry: &Path, profile: &Path) -> PathBuf {
    registry.join(format!(
        "{}.json",
        util::hash(profile.as_os_str().as_encoded_bytes())
    ))
}

pub(crate) fn register(registry: &Path, profile: &Path, endpoint: &str) -> AppResult<()> {
    util::atomic_file(
        &record_path(registry, profile),
        &serde_json::to_vec(&Record {
            profile: profile.to_owned(),
            endpoint: endpoint.to_owned(),
            owner_pid: std::process::id(),
        })?,
    )
}

pub(crate) fn forget(registry: &Path, profile: &Path) {
    let _ = fs::remove_file(record_path(registry, profile));
}

pub(crate) fn coordinate(registry: &Path) -> AppResult<fs::File> {
    util::private_dir(registry)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(registry.join("cleanup.lock"))?;
    lock.lock()?;
    Ok(lock)
}

pub(crate) fn remove_profile(profile: &Path) -> std::io::Result<()> {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match fs::remove_dir_all(profile) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::DirectoryNotEmpty
                ) && Instant::now() < until =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn cleanup(registry: &Path) -> AppResult<()> {
    let _coordination = coordinate(registry)?;
    let entries = match fs::read_dir(registry) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let root = registry.canonicalize()?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        // A record itda cannot make sense of, or a browser it cannot reach,
        // is left for the next cleanup; it does not keep the worker from
        // starting.
        let _ = clean(&root, &path);
    }
    Ok(())
}

/// Close the browser one registry record names and remove its profile and
/// the record, once the browser has shown it owns that profile.
fn clean(root: &Path, path: &Path) -> AppResult<()> {
    let record: Record = serde_json::from_slice(&util::read_bounded(path, 16 * 1024)?)?;
    if !record.profile.exists() {
        fs::remove_file(path)?;
        return Ok(());
    }
    let profile = record.profile.canonicalize()?;
    if !profile.starts_with(root.join("profiles")) {
        return Err(AppError::api("unsafe_target", 400));
    }
    let endpoint =
        url::Url::parse(&record.endpoint).map_err(|_| AppError::api("data_invalid", 400))?;
    if endpoint.scheme() != "ws"
        || endpoint.host_str() != Some("127.0.0.1")
        || !endpoint.path().starts_with("/devtools/browser/")
    {
        return Err(AppError::api("unsafe_target", 400));
    }
    let port = endpoint
        .port()
        .ok_or_else(|| AppError::api("data_invalid", 400))?;
    match TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(1)) {
        Ok(stream) => {
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
            if let Ok((mut socket, _)) = tungstenite::client(record.endpoint.as_str(), stream) {
                socket
                    .send(Message::text(
                        json!({"id":1,"method":"Browser.getBrowserCommandLine","params":{}})
                            .to_string(),
                    ))
                    .map_err(|_| AppError::api("browser_unavailable", 502))?;
                let until = Instant::now() + Duration::from_secs(1);
                let expected = format!("--user-data-dir={}", record.profile.display());
                let owned = loop {
                    if Instant::now() >= until {
                        break false;
                    }
                    let message = socket
                        .read()
                        .map_err(|_| AppError::api("browser_unavailable", 502))?;
                    if let Ok(text) = message.to_text() {
                        let value: serde_json::Value = serde_json::from_str(text)?;
                        if value["id"] == 1 {
                            break value["result"]["arguments"].as_array().is_some_and(|args| {
                                args.iter().any(|arg| arg.as_str() == Some(&expected))
                            });
                        }
                    }
                };
                if !owned {
                    return Err(AppError::api("browser_state_mismatch", 409));
                }
                socket
                    .send(Message::text(
                        json!({"id":2,"method":"Browser.close","params":{}}).to_string(),
                    ))
                    .map_err(|_| AppError::api("browser_unavailable", 502))?;
                let until = Instant::now() + Duration::from_secs(1);
                while Instant::now() < until {
                    match socket.read() {
                        Ok(Message::Close(_))
                        | Err(tungstenite::Error::ConnectionClosed)
                        | Err(tungstenite::Error::AlreadyClosed) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            } else {
                return Err(AppError::api("browser_unavailable", 502));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
        Err(_) => return Err(AppError::api("browser_unavailable", 502)),
    }
    remove_profile(&profile)?;
    fs::remove_file(path)?;
    remove_profile(&profile)?;
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_never_removes_a_profile_outside_its_registry() {
        let root = std::env::temp_dir().join(format!("itda-orphan-test-{}", util::id()));
        let registry = root.join("registry");
        let unrelated = root.join("unrelated");
        util::private_dir(&unrelated).unwrap();
        register(
            &registry,
            &unrelated,
            "ws://127.0.0.1:1/devtools/browser/test",
        )
        .unwrap();
        // The record is left alone, and does not keep cleanup from finishing.
        cleanup(&registry).unwrap();
        assert!(unrelated.exists());
        assert_eq!(
            fs::read_dir(&registry)
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "json"))
                .count(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_skips_a_record_it_cannot_read() {
        let root = std::env::temp_dir().join(format!("itda-orphan-test-{}", util::id()));
        let registry = root.join("registry");
        util::private_dir(&registry).unwrap();
        fs::write(registry.join("broken.json"), b"not json").unwrap();
        cleanup(&registry).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
