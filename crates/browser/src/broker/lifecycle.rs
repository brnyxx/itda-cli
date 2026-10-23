use super::{BrokerConfig, HANDSHAKE_TIMEOUT, STARTUP_TIMEOUT, connect, fail, remaining, wire};
use aws_lc_rs::digest;
use itda_types::error::{AppError, AppResult};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub(super) fn private_dir(path: &Path) -> AppResult<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| fail())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| fail())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(fail());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(AppError::api("browser_state_not_private", 403));
        }
    }
    Ok(())
}

pub(super) fn open_lock(path: &Path) -> AppResult<File> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(fail());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| fail())
}

pub(super) fn startup_lock(config: &BrokerConfig, timeout: Duration) -> AppResult<File> {
    let file = open_lock(&config.state_dir.join("startup.lock"))?;
    lock_file(file, timeout)
}

pub(super) fn lock_file(file: File, timeout: Duration) -> AppResult<File> {
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::Error(_)) => Err(fail()),
        Err(TryLockError::WouldBlock) => {
            let (sender, receiver) = mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let result = file.lock().map(|()| file).map_err(|_| fail());
                let _ = sender.send(result);
            });
            receiver
                .recv_timeout(timeout)
                .map_err(|_| AppError::api("timeout", 502))?
        }
    }
}

pub(super) fn owner_alive(config: &BrokerConfig) -> AppResult<bool> {
    let file = open_lock(&config.state_dir.join("owner.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(_)) => Err(fail()),
    }
}

pub(super) fn generation_ended(
    config: &BrokerConfig,
    previous: &wire::Endpoint,
    deadline: Instant,
) -> AppResult<bool> {
    // Looking at the owner lock takes it for an instant when it is free; under
    // the startup lock no new worker can be reaching for it at that moment.
    let _startup = startup_lock(config, remaining(deadline)?)?;
    if !owner_alive(config)? {
        return Ok(true);
    }
    let current = read_endpoint(config)?;
    if current.generation == previous.generation {
        return Ok(false);
    }
    check_identity(config, &current)?;
    probe(&current, remaining(deadline)?)?;
    Ok(true)
}

/// The identity of this executable, browser and address policy, hashed once
/// per process: the executable does not change while it runs.
pub(super) fn identity(config: &BrokerConfig) -> AppResult<String> {
    static IDENTITIES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, String>>,
    > = std::sync::OnceLock::new();
    let key = format!(
        "{}\0{:?}\0{}",
        config.executable.display(),
        config.browser,
        config.allow_local
    );
    let identities = IDENTITIES.get_or_init(Default::default);
    if let Some(identity) = identities
        .lock()
        .ok()
        .and_then(|map| map.get(&key).cloned())
    {
        return Ok(identity);
    }
    let identity = hash_identity(config)?;
    if let Ok(mut map) = identities.lock() {
        map.insert(key, identity.clone());
    }
    Ok(identity)
}

fn hash_identity(config: &BrokerConfig) -> AppResult<String> {
    let executable = fs::canonicalize(&config.executable).map_err(|_| fail())?;
    let mut file = File::open(&executable).map_err(|_| fail())?;
    let mut hash = digest::Context::new(&digest::SHA256);
    hash.update(executable.as_os_str().as_encoded_bytes());
    hash.update(&[0]);
    if let Some(browser) = &config.browser {
        let browser = fs::canonicalize(browser).map_err(|_| fail())?;
        hash.update(browser.as_os_str().as_encoded_bytes());
    }
    hash.update(&[u8::from(config.allow_local)]);
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| fail())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) fn read_endpoint(config: &BrokerConfig) -> AppResult<wire::Endpoint> {
    let path = config.state_dir.join("endpoint.json");
    let metadata = fs::symlink_metadata(&path).map_err(|_| fail())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > wire::META_LIMIT as u64
    {
        return Err(fail());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(fail());
        }
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| fail())?
        .take((wire::META_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| fail())?;
    if bytes.len() > wire::META_LIMIT {
        return Err(fail());
    }
    serde_json::from_slice(&bytes).map_err(|_| fail())
}

pub(super) fn check_identity(config: &BrokerConfig, endpoint: &wire::Endpoint) -> AppResult<()> {
    if endpoint.protocol != wire::PROTOCOL
        || endpoint.identity != identity(config)?
        || endpoint.allow_local != config.allow_local
    {
        return Err(AppError::api("browser_broker_mismatch", 409));
    }
    Ok(())
}

fn probe(endpoint: &wire::Endpoint, timeout: Duration) -> AppResult<()> {
    let mut stream = connect(endpoint, timeout.min(HANDSHAKE_TIMEOUT))?;
    wire::client_auth(&mut stream, endpoint)?;
    let mut stream = wire::TimedStream::new(&mut stream, timeout.min(HANDSHAKE_TIMEOUT))?;
    wire::Frame::encode(&wire::Request::Ping, &[])?
        .write(&mut stream)
        .map_err(|_| fail())?;
    let (reply, body) = wire::read_frame::<wire::Reply>(&mut stream)?;
    if !matches!(reply, wire::Reply::Ack) || !body.is_empty() {
        return Err(fail());
    }
    Ok(())
}

pub(super) fn endpoint(config: &BrokerConfig, deadline: Instant) -> AppResult<wire::Endpoint> {
    endpoint_with(config, deadline, start)
}

pub(super) fn endpoint_with(
    config: &BrokerConfig,
    deadline: Instant,
    start: impl FnOnce(&BrokerConfig, Instant) -> AppResult<wire::Endpoint>,
) -> AppResult<wire::Endpoint> {
    private_dir(&config.state_dir)?;
    let _startup = startup_lock(config, remaining(deadline)?.min(STARTUP_TIMEOUT))?;
    if owner_alive(config)? {
        let endpoint = read_endpoint(config)?;
        check_identity(config, &endpoint)?;
        return Ok(endpoint);
    }
    start(config, deadline)
}

fn start(config: &BrokerConfig, deadline: Instant) -> AppResult<wire::Endpoint> {
    let mut command = Command::new(&config.executable);
    command
        .arg("__browser-worker")
        .arg("--state-dir")
        .arg(&config.state_dir);
    if config.allow_local {
        command.arg("--allow-local");
    }
    if let Some(browser) = &config.browser {
        command.arg("--browser").arg(browser);
        command.env("ITDA_BROWSER", browser);
    }
    command
        .env("ITDA_BROWSER_REGISTRY", config.state_dir.join("chrome"))
        .env("ITDA_BROWSER_SUPERVISOR", &config.executable)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0200 | 0x0000_0008);
    }
    let mut child = command.spawn().map_err(|_| fail())?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(fail());
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(wire::read_frame::<wire::Ready>(&mut stdout));
    });
    let startup = (|| {
        let (ready, body) = receiver
            .recv_timeout(remaining(deadline)?.min(STARTUP_TIMEOUT))
            .map_err(|_| AppError::api("timeout", 502))??;
        let endpoint = read_endpoint(config)?;
        if ready.protocol != wire::PROTOCOL
            || ready.generation != endpoint.generation
            || !body.is_empty()
        {
            return Err(fail());
        }
        check_identity(config, &endpoint)?;
        probe(&endpoint, remaining(deadline)?)?;
        Ok(endpoint)
    })();
    if startup.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    } else {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    startup
}

pub(super) fn publish(config: &BrokerConfig, endpoint: &wire::Endpoint) -> AppResult<()> {
    let temporary = config
        .state_dir
        .join(format!("endpoint-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| fail())?;
    let result = (|| {
        let bytes = serde_json::to_vec(endpoint).map_err(|_| fail())?;
        file.write_all(&bytes).map_err(|_| fail())?;
        file.sync_all().map_err(|_| fail())?;
        drop(file);
        let destination = config.state_dir.join("endpoint.json");
        #[cfg(windows)]
        if destination.exists() {
            fs::remove_file(&destination).map_err(|_| fail())?;
        }
        fs::rename(&temporary, destination).map_err(|_| fail())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(super) struct Publication(pub PathBuf);

impl Drop for Publication {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
