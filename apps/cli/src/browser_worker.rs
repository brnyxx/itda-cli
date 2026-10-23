use itda_browser::broker::{BrokerConfig, BrowserClient};
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use std::path::{Path, PathBuf};

pub fn config(home: &Path, allow_local: bool) -> AppResult<BrokerConfig> {
    let browser = std::env::var_os("ITDA_BROWSER").map(PathBuf::from);
    let executable = std::env::current_exe()?;
    let metadata = std::fs::metadata(&executable)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| AppError::api("internal", 500))?
        .as_nanos();
    let identity = util::hash(&serde_json::to_vec(&(
        home.canonicalize()?,
        &browser,
        allow_local,
        &executable,
        metadata.len(),
        modified,
    ))?);
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .ok_or_else(|| AppError::api("internal", 500))?
        .join("itda")
        .join("browser-runtime");
    #[cfg(not(windows))]
    let base = home.join("browser-runtime");
    let state_dir = base.join(identity);
    sweep(&base, &state_dir);
    Ok(BrokerConfig {
        state_dir,
        executable,
        browser,
        allow_local,
    })
}

/// Remove the runtime directories of itda builds that are gone. Each upgrade
/// leaves one behind, with a worker secret and lock files in it. A directory
/// goes only when nothing has touched it for a day and no worker holds its
/// owner lock; one that is just being set up, or still serves an older itda,
/// stays.
fn sweep(base: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let idle = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(24 * 60 * 60));
        if !idle {
            continue;
        }
        let owner = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join("owner.lock"));
        let _held = match owner {
            Ok(file) => match file.try_lock() {
                Ok(()) => Some(file),
                Err(_) => continue,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => continue,
        };
        let _ = std::fs::remove_dir_all(&path);
    }
}

pub fn client(home: &Path, allow_local: bool) -> AppResult<BrowserClient> {
    Ok(BrowserClient::broker(config(home, allow_local)?))
}

pub async fn reset(home: &Path, origin: &str, account: &str) -> AppResult<()> {
    for allow_local in [false, true] {
        let client = client(home, allow_local)?;
        let (origin, account) = (origin.to_owned(), account.to_owned());
        tokio::task::spawn_blocking(move || client.reset(&origin, &account))
            .await
            .map_err(|_| AppError::api("internal", 500))??;
    }
    Ok(())
}

pub fn stop(home: &Path) -> AppResult<()> {
    if !home.try_exists()? {
        return Ok(());
    }
    for allow_local in [false, true] {
        client(home, allow_local)?.shutdown()?;
    }
    Ok(())
}
