use crate::orphans;
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

pub fn run(browser: &Path, profile: &Path, arguments: &[OsString]) -> AppResult<()> {
    let registry = std::env::var_os("ITDA_BROWSER_REGISTRY")
        .map(PathBuf::from)
        .ok_or_else(|| AppError::api("unsafe_target", 400))?;
    let owned = profile.canonicalize()?;
    let root = registry.join("profiles").canonicalize()?;
    let expected = OsString::from(format!("--user-data-dir={}", profile.display()));
    if owned.parent() != Some(root.as_path())
        || std::fs::symlink_metadata(profile)?.file_type().is_symlink()
        || !profile
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("itda-browser-"))
        || !arguments.contains(&expected)
    {
        return Err(AppError::api("unsafe_target", 400));
    }
    let guards = registry.join("guardians");
    util::private_dir(&guards)?;
    let guard_path = guards.join(format!(
        "{}.lock",
        util::hash(profile.as_os_str().as_encoded_bytes())
    ));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let guard = options.open(&guard_path)?;
    guard.lock()?;
    let (lost, parent) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("browser-lifeline".into())
        .spawn(move || {
            let mut input = std::io::stdin().lock();
            let mut byte = [0];
            while matches!(input.read(&mut byte), Ok(1)) {}
            let _ = lost.send(());
        })?;
    let mut child = Command::new(browser)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    let outcome = if let Some(child) = child.as_mut() {
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break Ok(()),
                Err(error) => break Err(AppError::from(error)),
                Ok(None) => {}
            }
            match parent.recv_timeout(Duration::from_millis(100)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    } else {
        Err(AppError::api("browser_unavailable", 502))
    };
    let cleanup = {
        let coordination = orphans::coordinate(&registry);
        if let Some(mut child) = child {
            let _ = child.kill();
            let _ = child.wait();
        }
        coordination.and_then(|_coordination| {
            orphans::remove_profile(profile)?;
            orphans::forget(&registry, profile);
            Ok(())
        })
    };
    drop(guard);
    let _ = std::fs::remove_file(guard_path);
    outcome?;
    cleanup
}
