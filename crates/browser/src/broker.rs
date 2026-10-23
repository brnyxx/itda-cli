use crate::transport::{BrowserRequest, BrowserResponse, BrowserTransport};
use itda_types::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

mod lifecycle;
mod pool;
mod wire;

#[cfg(test)]
mod tests;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const RESET_TIMEOUT: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct BrokerConfig {
    pub state_dir: PathBuf,
    pub executable: PathBuf,
    pub browser: Option<PathBuf>,
    pub allow_local: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCompletion {
    NotStarted,
    Quiescent,
    Unknown,
}

#[derive(Debug)]
pub struct BrowserFailure {
    pub error: AppError,
    pub completion: BrowserCompletion,
}

impl BrowserFailure {
    fn not_started(error: AppError) -> Self {
        Self {
            error,
            completion: BrowserCompletion::NotStarted,
        }
    }

    fn unknown(error: AppError) -> Self {
        Self {
            error,
            completion: BrowserCompletion::Unknown,
        }
    }
}

#[derive(Clone)]
pub struct BrowserClient {
    inner: Arc<Client>,
}

enum Client {
    Local(pool::Pool<BrowserTransport>),
    Broker(BrokerConfig),
}

impl BrowserClient {
    pub fn local() -> Self {
        Self {
            inner: Arc::new(Client::Local(pool::Pool::new(BrowserTransport::new))),
        }
    }

    pub fn broker(config: BrokerConfig) -> Self {
        Self {
            inner: Arc::new(Client::Broker(config)),
        }
    }

    pub fn execute(&self, mut request: BrowserRequest) -> Result<BrowserResponse, BrowserFailure> {
        match &*self.inner {
            Client::Local(pool) => pool.execute(request),
            Client::Broker(config) => {
                let deadline = deadline(request.timeout).map_err(BrowserFailure::not_started)?;
                wire::Frame::encode(
                    &wire::Request::execute(&request),
                    request.body.as_deref().unwrap_or_default(),
                )
                .map_err(BrowserFailure::not_started)?;
                let endpoint =
                    lifecycle::endpoint(config, deadline).map_err(BrowserFailure::not_started)?;
                request.timeout = remaining(deadline).map_err(BrowserFailure::not_started)?;
                match remote_execute(&endpoint, request) {
                    Err(mut failure)
                        if failure.completion == BrowserCompletion::Unknown
                            && lifecycle::generation_ended(config, &endpoint, deadline)
                                .is_ok_and(|ended| ended) =>
                    {
                        failure.completion = BrowserCompletion::Quiescent;
                        failure.error = failure.error.with("broker_gone", true);
                        Err(failure)
                    }
                    result => result,
                }
            }
        }
    }

    pub fn reset(&self, origin: &str, account: &str) -> AppResult<()> {
        match &*self.inner {
            Client::Local(pool) => pool.reset(origin, account, RESET_TIMEOUT),
            Client::Broker(config) => {
                if !config.state_dir.exists() {
                    return Ok(());
                }
                lifecycle::private_dir(&config.state_dir)?;
                let startup = lifecycle::startup_lock(config, STARTUP_TIMEOUT)?;
                if !lifecycle::owner_alive(config)? {
                    return Ok(());
                }
                let endpoint = lifecycle::read_endpoint(config)?;
                lifecycle::check_identity(config, &endpoint)?;
                drop(startup);
                let mut stream = connect(&endpoint, HANDSHAKE_TIMEOUT)?;
                wire::client_auth(&mut stream, &endpoint)?;
                let mut stream =
                    wire::TimedStream::new(&mut stream, RESET_TIMEOUT + HANDSHAKE_TIMEOUT)?;
                wire::Frame::encode(
                    &wire::Request::Reset {
                        origin: origin.into(),
                        account: account.into(),
                    },
                    &[],
                )?
                .write(&mut stream)
                .map_err(|_| fail())?;
                let (reply, body) = wire::read_frame::<wire::Reply>(&mut stream)?;
                match reply {
                    wire::Reply::Ack if body.is_empty() => Ok(()),
                    other => Err(other
                        .into_result(body)
                        .err()
                        .map(|e| e.error)
                        .unwrap_or_else(fail)),
                }
            }
        }
    }

    pub fn shutdown(&self) -> AppResult<()> {
        let config = match &*self.inner {
            Client::Local(pool) => return pool.shutdown(RESET_TIMEOUT, false),
            Client::Broker(config) => config,
        };
        if !config.state_dir.exists() {
            return Ok(());
        }
        let deadline = deadline(RESET_TIMEOUT + HANDSHAKE_TIMEOUT * 2)?;
        lifecycle::private_dir(&config.state_dir)?;
        let _startup = lifecycle::startup_lock(config, STARTUP_TIMEOUT)?;
        if !lifecycle::owner_alive(config)? {
            return Ok(());
        }
        let endpoint = lifecycle::read_endpoint(config)?;
        lifecycle::check_identity(config, &endpoint)?;
        let mut socket = connect(&endpoint, remaining(deadline)?.min(HANDSHAKE_TIMEOUT))?;
        wire::client_auth(&mut socket, &endpoint)?;
        let mut stream = wire::TimedStream::new(&mut socket, remaining(deadline)?)?;
        wire::Frame::encode(&wire::Request::Shutdown, &[])?
            .write(&mut stream)
            .map_err(|_| fail())?;
        let (reply, body) = wire::read_frame::<wire::Reply>(&mut stream)?;
        match reply {
            wire::Reply::Ack if body.is_empty() => {
                let owner = lifecycle::open_lock(&config.state_dir.join("owner.lock"))?;
                drop(lifecycle::lock_file(owner, remaining(deadline)?)?);
                Ok(())
            }
            other => Err(other
                .into_result(body)
                .err()
                .map(|failure| failure.error)
                .unwrap_or_else(fail)),
        }
    }
}

fn remote_execute(
    endpoint: &wire::Endpoint,
    request: BrowserRequest,
) -> Result<BrowserResponse, BrowserFailure> {
    let deadline = deadline(request.timeout).map_err(BrowserFailure::not_started)?;
    let mut stream = connect(endpoint, request.timeout.min(HANDSHAKE_TIMEOUT))
        .map_err(BrowserFailure::not_started)?;
    wire::client_auth(&mut stream, endpoint).map_err(BrowserFailure::not_started)?;
    let left = remaining(deadline).map_err(BrowserFailure::not_started)?;
    let mut request = request;
    request.timeout = left;
    let frame = wire::Frame::encode(
        &wire::Request::execute(&request),
        request.body.as_deref().unwrap_or_default(),
    )
    .map_err(BrowserFailure::not_started)?;
    let mut stream = wire::TimedStream::new(&mut stream, left + HANDSHAKE_TIMEOUT)
        .map_err(BrowserFailure::not_started)?;
    frame
        .write(&mut stream)
        .map_err(|_| BrowserFailure::unknown(fail()))?;
    let (reply, body) =
        wire::read_frame::<wire::Reply>(&mut stream).map_err(BrowserFailure::unknown)?;
    reply.into_result(body)
}

fn connect(endpoint: &wire::Endpoint, timeout: Duration) -> AppResult<TcpStream> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let stream = TcpStream::connect_timeout(&address, timeout).map_err(|_| fail())?;
    set_timeout(&stream, timeout)?;
    Ok(stream)
}

fn set_timeout(stream: &TcpStream, timeout: Duration) -> AppResult<()> {
    stream.set_read_timeout(Some(timeout)).map_err(|_| fail())?;
    stream.set_write_timeout(Some(timeout)).map_err(|_| fail())
}

fn fail() -> AppError {
    AppError::api("browser_unavailable", 502)
}

fn deadline(timeout: Duration) -> AppResult<Instant> {
    if timeout.is_zero() {
        return Err(AppError::api("timeout", 502));
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| AppError::api("data_invalid", 400))
}

fn remaining(deadline: Instant) -> AppResult<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| AppError::api("timeout", 502))
}

pub fn run(config: BrokerConfig, ready: impl Write) -> AppResult<()> {
    run_with(config, ready, IDLE_TIMEOUT, BrowserTransport::new)
}

struct Activity {
    active: usize,
    touched: Instant,
    stopping: bool,
    #[cfg(test)]
    idle_waiting: bool,
}

struct Service {
    state: Mutex<Activity>,
    changed: Condvar,
}

impl Service {
    fn wait_until_idle(&self, idle: Duration) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if state.stopping {
                return false;
            }
            if state.active != 0 {
                #[cfg(test)]
                {
                    state.idle_waiting = true;
                    self.changed.notify_all();
                }
                state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
                #[cfg(test)]
                {
                    state.idle_waiting = false;
                }
                continue;
            }
            let left = idle.saturating_sub(state.touched.elapsed());
            if left.is_zero() {
                state.stopping = true;
                return true;
            }
            (state, _) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner());
        }
    }
}

struct Active(Arc<Service>);

impl Drop for Active {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        state.active -= 1;
        state.touched = Instant::now();
        self.0.changed.notify_all();
    }
}

fn run_with<D: pool::Driver>(
    config: BrokerConfig,
    mut ready: impl Write,
    idle: Duration,
    factory: impl Fn() -> D + Send + Sync + 'static,
) -> AppResult<()> {
    lifecycle::private_dir(&config.state_dir)?;
    let owner = lifecycle::open_lock(&config.state_dir.join("owner.lock"))?;
    owner.try_lock().map_err(|_| fail())?;
    crate::orphans::cleanup(&config.state_dir.join("chrome"))?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| fail())?;
    let address = listener.local_addr().map_err(|_| fail())?;
    let endpoint = wire::Endpoint::new(
        address.port(),
        lifecycle::identity(&config)?,
        config.allow_local,
    )?;
    lifecycle::publish(&config, &endpoint)?;
    let publication = lifecycle::Publication(config.state_dir.join("endpoint.json"));
    wire::Frame::encode(
        &wire::Ready {
            protocol: wire::PROTOCOL,
            generation: endpoint.generation,
        },
        &[],
    )?
    .write(&mut ready)
    .map_err(|_| fail())?;
    drop(ready);
    let service = Arc::new(Service {
        state: Mutex::new(Activity {
            active: 0,
            touched: Instant::now(),
            stopping: false,
            #[cfg(test)]
            idle_waiting: false,
        }),
        changed: Condvar::new(),
    });
    let pool = Arc::new(pool::Pool::new(factory));
    let endpoint = Arc::new(endpoint);
    let timer = {
        let service = service.clone();
        std::thread::spawn(move || {
            if service.wait_until_idle(idle) {
                let _ = TcpStream::connect_timeout(&address, HANDSHAKE_TIMEOUT);
            }
        })
    };
    let result = loop {
        let (stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(_) => break Err(fail()),
        };
        let mut state = service.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.stopping {
            break Ok(());
        }
        if state.active == 32 {
            drop(state);
            drop(stream);
            continue;
        }
        state.active += 1;
        drop(state);
        let active = Active(service.clone());
        let pool = pool.clone();
        let endpoint = endpoint.clone();
        let service = service.clone();
        std::thread::spawn(move || {
            let _active = active;
            let _ = serve(stream, &endpoint, &pool, &service);
        });
    };
    {
        let mut state = service.state.lock().unwrap_or_else(|p| p.into_inner());
        state.stopping = true;
        service.changed.notify_all();
        while state.active != 0 {
            state = service
                .changed
                .wait(state)
                .unwrap_or_else(|p| p.into_inner());
        }
    }
    timer.join().map_err(|_| fail())?;
    drop(pool);
    drop(publication);
    drop(owner);
    result
}

fn serve<D: pool::Driver>(
    mut stream: TcpStream,
    endpoint: &wire::Endpoint,
    pool: &pool::Pool<D>,
    service: &Service,
) -> AppResult<()> {
    set_timeout(&stream, HANDSHAKE_TIMEOUT)?;
    wire::server_auth(&mut stream, endpoint)?;
    let received = Instant::now();
    let (request, body) = wire::read_frame::<wire::Request>(&mut wire::TimedStream::new(
        &mut stream,
        HANDSHAKE_TIMEOUT,
    )?)?;
    let (reply, bytes) = match request {
        wire::Request::Ping if body.is_empty() => (wire::Reply::Ack, Vec::new()),
        wire::Request::Shutdown if body.is_empty() => {
            {
                let mut state = service.state.lock().unwrap_or_else(|p| p.into_inner());
                state.stopping = true;
                service.changed.notify_all();
            }
            let _ = TcpStream::connect_timeout(
                &SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port)),
                HANDSHAKE_TIMEOUT,
            );
            let reply = match pool.shutdown(RESET_TIMEOUT, true) {
                Ok(()) => wire::Reply::Ack,
                Err(error) => wire::Reply::failure(BrowserFailure::unknown(error)),
            };
            (reply, Vec::new())
        }
        wire::Request::Reset { origin, account } if body.is_empty() => {
            let reply = match pool.reset(&origin, &account, RESET_TIMEOUT) {
                Ok(()) => wire::Reply::Ack,
                Err(error) => wire::Reply::failure(BrowserFailure::unknown(error)),
            };
            (reply, Vec::new())
        }
        request @ wire::Request::Execute { .. } => {
            let result = request
                .into_browser(body)
                .map_err(BrowserFailure::not_started)
                .and_then(|mut request| {
                    if request.allow_local && !endpoint.allow_local {
                        return Err(BrowserFailure::not_started(AppError::api(
                            "data_invalid",
                            400,
                        )));
                    }
                    request.timeout = request.timeout.saturating_sub(received.elapsed());
                    pool.execute(request)
                });
            match result {
                Ok(response) => (
                    wire::Reply::Response {
                        url: response.url,
                        status: response.status,
                        headers: response.headers,
                        resource_limits: response.resource_limits,
                    },
                    response.bytes,
                ),
                Err(error) => (wire::Reply::failure(error), Vec::new()),
            }
        }
        _ => (
            wire::Reply::failure(BrowserFailure::not_started(AppError::api(
                "data_invalid",
                400,
            ))),
            Vec::new(),
        ),
    };
    let frame = wire::Frame::encode(&reply, &bytes).or_else(|error| {
        wire::Frame::encode(
            &wire::Reply::failure(BrowserFailure {
                error,
                completion: BrowserCompletion::Quiescent,
            }),
            &[],
        )
    })?;
    frame
        .write(&mut wire::TimedStream::new(&mut stream, HANDSHAKE_TIMEOUT)?)
        .map_err(|_| fail())
}
