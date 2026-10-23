use super::{
    Active, Activity, BrokerConfig, BrowserClient, BrowserCompletion, Service, lifecycle, pool,
    remote_execute, run_with, wire,
};
use crate::transport::{BrowserRequest, BrowserResponse};
use itda_types::error::AppResult;
use std::{
    fs,
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Barrier, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn request(account: &str) -> BrowserRequest {
    BrowserRequest {
        url: "http://127.0.0.1:12345/fixture".into(),
        method: "GET".into(),
        headers: vec![("authorization".into(), "fixture-secret".into())],
        body: None,
        account_id: account.into(),
        credential_fingerprint: Some("stable".into()),
        read_only: true,
        cooldowns: Vec::new(),
        limit: 1024,
        timeout: Duration::from_secs(5),
        allow_local: true,
        mode: crate::transport::BrowserMode::Fetch,
    }
}

/// The worker gave its owner lock back. A process another test spawns at that
/// moment holds a copy of the lock's descriptor until it execs, so the lock
/// can stay taken for an instant after the worker let go; wait for it, briefly.
fn owner_released(cfg: &BrokerConfig) {
    let file = lifecycle::open_lock(&cfg.state_dir.join("owner.lock")).expect("owner lock");
    drop(lifecycle::lock_file(file, Duration::from_secs(5)).expect("owner released"));
}

fn config() -> BrokerConfig {
    BrokerConfig {
        state_dir: std::env::temp_dir().join(format!("itda-broker-{}", uuid::Uuid::new_v4())),
        executable: std::env::current_exe().expect("test executable"),
        browser: None,
        allow_local: true,
    }
}

struct Fixture {
    calls: usize,
    entered: mpsc::Sender<String>,
    release: Option<Arc<Barrier>>,
}

impl pool::Driver for Fixture {
    fn execute(&mut self, request: BrowserRequest) -> AppResult<BrowserResponse> {
        self.calls += 1;
        self.entered.send(request.account_id).expect("entered");
        if let Some(release) = self.release.take() {
            release.wait();
        }
        Ok(BrowserResponse {
            url: request.url,
            status: 200,
            headers: Vec::new(),
            bytes: self.calls.to_be_bytes().to_vec(),
            resource_limits: Vec::new(),
        })
    }
}

#[test]
fn pool_overlaps_keys_and_serializes_one_key() {
    let (entered, calls) = mpsc::channel();
    let release = Arc::new(Barrier::new(3));
    let pool = Arc::new(pool::Pool::new({
        let release = release.clone();
        move || Fixture {
            calls: 0,
            entered: entered.clone(),
            release: Some(release.clone()),
        }
    }));
    let a = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("a")))
    };
    assert_eq!(calls.recv_timeout(Duration::from_secs(5)).expect("a"), "a");
    let queued = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("a")))
    };
    pool.wait_for_waiters(1);
    let b = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("b")))
    };
    assert_eq!(calls.recv_timeout(Duration::from_secs(5)).expect("b"), "b");
    assert!(calls.try_recv().is_err());
    release.wait();
    assert_eq!(
        a.join().expect("join").expect("a").bytes,
        1_usize.to_be_bytes()
    );
    assert_eq!(
        b.join().expect("join").expect("b").bytes,
        1_usize.to_be_bytes()
    );
    assert_eq!(
        queued.join().expect("join").expect("queued").bytes,
        2_usize.to_be_bytes()
    );
}

#[test]
fn reset_invalidates_waiters_and_waits_for_active_cleanup() {
    let (entered, calls) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let pool = Arc::new(pool::Pool::new({
        let release = release.clone();
        move || Fixture {
            calls: 0,
            entered: entered.clone(),
            release: Some(release.clone()),
        }
    }));
    let active = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("a")))
    };
    calls.recv_timeout(Duration::from_secs(5)).expect("active");
    let queued = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("a")))
    };
    pool.wait_for_waiters(1);
    let (ack, acknowledged) = mpsc::channel();
    let reset = {
        let pool = pool.clone();
        std::thread::spawn(move || {
            let result = pool.reset("http://127.0.0.1:12345", "a", Duration::from_secs(5));
            ack.send(()).expect("ack");
            result
        })
    };
    assert_eq!(
        queued
            .join()
            .expect("join")
            .expect_err("invalidated")
            .completion,
        BrowserCompletion::NotStarted
    );
    assert!(acknowledged.try_recv().is_err());
    release.wait();
    active.join().expect("join").expect("active");
    reset.join().expect("join").expect("reset");
}

#[test]
fn framing_preserves_raw_binary_and_rejects_oversized_lengths() {
    let bytes = vec![0, 255, 128, b'\n', b'\r'];
    let frame = wire::Frame::encode(&serde_json::json!({"fixture": true}), &bytes).expect("encode");
    let mut stream = Cursor::new(Vec::new());
    frame.write(&mut stream).expect("write");
    let encoded = stream.into_inner();
    assert!(encoded.ends_with(&bytes));
    let (meta, body): (serde_json::Value, Vec<u8>) =
        wire::read_frame(&mut Cursor::new(encoded)).expect("read");
    assert_eq!(meta["fixture"], true);
    assert_eq!(body, bytes);
    let mut oversized = Cursor::new([u32::MAX.to_be_bytes(), 0_u32.to_be_bytes()].concat());
    assert!(wire::read_frame::<serde_json::Value>(&mut oversized).is_err());
}

#[test]
fn request_wire_preserves_the_callers_timeout_and_empty_body() {
    let mut request = request("a");
    request.timeout = Duration::from_secs(3600);
    request.body = Some(Vec::new());
    let decoded = wire::Request::execute(&request)
        .into_browser(Vec::new())
        .expect("request");
    assert_eq!(decoded.timeout, request.timeout);
    assert_eq!(decoded.body, Some(Vec::new()));
}

#[test]
fn failed_server_authentication_receives_no_request_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let endpoint = wire::Endpoint::fixture(listener.local_addr().expect("address").port());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut hello = [0; wire::HELLO_LEN];
        stream.read_exact(&mut hello).expect("hello");
        stream.write_all(&[0; 64]).expect("fake proof");
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).expect("closed");
        assert!(
            rest.is_empty(),
            "credentials escaped before server authentication"
        );
    });
    let mut stream = TcpStream::connect(("127.0.0.1", endpoint.port)).expect("connect");
    assert!(wire::client_auth(&mut stream, &endpoint).is_err());
    drop(stream);
    server.join().expect("server");
}

#[test]
fn client_authentication_rejects_wrong_key_and_replayed_generation() {
    for generation_mismatch in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let endpoint = wire::Endpoint::fixture(listener.local_addr().expect("address").port());
        let mut bad = endpoint.clone();
        if generation_mismatch {
            bad.generation[0] ^= 1;
        } else {
            bad.secret[0] ^= 1;
        }
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            assert!(wire::server_auth(&mut stream, &endpoint).is_err());
        });
        let mut stream = TcpStream::connect(("127.0.0.1", bad.port)).expect("connect");
        assert!(wire::client_auth(&mut stream, &bad).is_err());
        drop(stream);
        server.join().expect("server");
    }
}

#[test]
fn lost_reply_after_authenticated_send_is_unknown_without_replay() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let endpoint = wire::Endpoint::fixture(listener.local_addr().expect("address").port());
    let peer = endpoint.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        wire::server_auth(&mut stream, &peer).expect("auth");
        let (_, body) = wire::read_frame::<wire::Request>(&mut stream).expect("request");
        assert!(body.is_empty());
    });
    let error = remote_execute(&endpoint, request("a")).expect_err("lost reply");
    assert_eq!(error.completion, BrowserCompletion::Unknown);
    server.join().expect("server");
}

#[test]
fn startup_lock_serializes_contenders_without_replacing_lock_inode() {
    let cfg = config();
    fs::create_dir_all(&cfg.state_dir).expect("directory");
    let first = lifecycle::startup_lock(&cfg, Duration::from_secs(5)).expect("first lock");
    let (entered, attempted) = mpsc::channel();
    let second = {
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            entered.send(()).expect("attempt");
            lifecycle::startup_lock(&cfg, Duration::from_secs(5))
        })
    };
    attempted
        .recv_timeout(Duration::from_secs(5))
        .expect("started");
    let lock = lifecycle::open_lock(&cfg.state_dir.join("startup.lock")).expect("open");
    assert!(matches!(lock.try_lock(), Err(fs::TryLockError::WouldBlock)));
    drop(first);
    drop(second.join().expect("join").expect("second lock"));
    assert!(cfg.state_dir.join("startup.lock").exists());
    fs::remove_dir_all(&cfg.state_dir).expect("cleanup");
}

#[test]
fn simultaneous_startup_has_one_owner_and_one_generation() {
    let cfg = config();
    let barrier = Arc::new(Barrier::new(8));
    let starts = Arc::new(AtomicUsize::new(0));
    let (worker, workers) = mpsc::channel();
    let clients: Vec<_> = (0..8)
        .map(|_| {
            let cfg = cfg.clone();
            let barrier = barrier.clone();
            let starts = starts.clone();
            let worker = worker.clone();
            std::thread::spawn(move || {
                barrier.wait();
                lifecycle::endpoint_with(
                    &cfg,
                    Instant::now() + Duration::from_secs(10),
                    |cfg, _| {
                        starts.fetch_add(1, Ordering::SeqCst);
                        let (ready, readiness) = mpsc::channel();
                        let (entered, _calls) = mpsc::channel();
                        let server = {
                            let cfg = cfg.clone();
                            std::thread::spawn(move || {
                                run_with(
                                    cfg,
                                    ChannelWriter(ready),
                                    Duration::from_secs(60),
                                    move || Fixture {
                                        calls: 0,
                                        entered: entered.clone(),
                                        release: None,
                                    },
                                )
                            })
                        };
                        worker.send(server).expect("worker");
                        readiness
                            .recv_timeout(Duration::from_secs(5))
                            .expect("ready");
                        lifecycle::read_endpoint(cfg)
                    },
                )
                .expect("endpoint")
                .generation
            })
        })
        .collect();
    let generations: Vec<_> = clients
        .into_iter()
        .map(|client| client.join().expect("client"))
        .collect();
    assert!(
        generations
            .iter()
            .all(|generation| *generation == generations[0])
    );
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    BrowserClient::broker(cfg.clone())
        .shutdown()
        .expect("shutdown");
    drop(worker);
    for worker in workers {
        worker.join().expect("worker").expect("service");
    }
    fs::remove_dir_all(&cfg.state_dir).expect("cleanup");
}

struct CountedDriver {
    live: Arc<AtomicUsize>,
    entered: mpsc::Sender<String>,
    release: Option<Arc<Barrier>>,
}

impl Drop for CountedDriver {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl pool::Driver for CountedDriver {
    fn execute(&mut self, request: BrowserRequest) -> AppResult<BrowserResponse> {
        self.entered.send(request.account_id).expect("entered");
        if let Some(release) = self.release.take() {
            release.wait();
        }
        Ok(BrowserResponse {
            url: request.url,
            status: 200,
            headers: Vec::new(),
            bytes: Vec::new(),
            resource_limits: Vec::new(),
        })
    }
}

#[test]
fn fifth_key_waits_for_capacity_and_evicts_only_an_idle_driver() {
    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (entered, calls) = mpsc::channel();
    let release = Arc::new(Barrier::new(5));
    let created = Arc::new(AtomicUsize::new(0));
    let pool = Arc::new(pool::Pool::new({
        let live = live.clone();
        let peak = peak.clone();
        let release = release.clone();
        move || {
            let count = live.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(count, Ordering::SeqCst);
            CountedDriver {
                live: live.clone(),
                entered: entered.clone(),
                release: (created.fetch_add(1, Ordering::SeqCst) < 4).then(|| release.clone()),
            }
        }
    }));
    let mut threads = Vec::new();
    for index in 0..4 {
        let pool = pool.clone();
        threads.push(std::thread::spawn(move || {
            pool.execute(request(&index.to_string()))
        }));
    }
    for _ in 0..4 {
        calls
            .recv_timeout(Duration::from_secs(5))
            .expect("four active");
    }
    let fifth = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("fifth")))
    };
    pool.wait_for_waiters(1);
    assert_eq!(live.load(Ordering::SeqCst), 4);
    assert!(calls.try_recv().is_err());
    release.wait();
    for thread in threads {
        thread.join().expect("join").expect("request");
    }
    fifth.join().expect("join").expect("fifth");
    assert_eq!(peak.load(Ordering::SeqCst), 4);
    drop(pool);
    assert_eq!(live.load(Ordering::SeqCst), 0);
}

#[test]
fn reset_timeout_does_not_acknowledge_active_execution_as_clean() {
    let (entered, calls) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let pool = Arc::new(pool::Pool::new({
        let release = release.clone();
        move || Fixture {
            calls: 0,
            entered: entered.clone(),
            release: Some(release.clone()),
        }
    }));
    let active = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("a")))
    };
    calls.recv_timeout(Duration::from_secs(5)).expect("active");
    let result = pool.reset("http://127.0.0.1:12345", "a", Duration::from_millis(1));
    assert_eq!(result.expect_err("timeout").public_code, "timeout");
    release.wait();
    active.join().expect("join").expect("active");
}

#[test]
fn process_client() {
    let Some(dir) = std::env::var_os("ITDA_BROKER_TEST_STATE") else {
        return;
    };
    let client = BrowserClient::broker(BrokerConfig {
        state_dir: dir.into(),
        executable: std::env::current_exe().expect("executable"),
        browser: None,
        allow_local: true,
    });
    let response = client.execute(request("shared")).expect("response");
    let expected: usize = std::env::var("ITDA_BROKER_TEST_COUNT")
        .expect("count")
        .parse()
        .expect("number");
    assert_eq!(response.bytes, expected.to_be_bytes());
}

#[test]
fn independent_client_processes_share_one_session_and_shutdown_releases_owner() {
    let cfg = config();
    let (entered, calls) = mpsc::channel();
    let (ready, readiness) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let created = AtomicUsize::new(0);
    let worker = {
        let cfg = cfg.clone();
        let release = release.clone();
        std::thread::spawn(move || {
            run_with(
                cfg,
                ChannelWriter(ready),
                Duration::from_secs(60),
                move || Fixture {
                    calls: 0,
                    entered: entered.clone(),
                    release: (created.fetch_add(1, Ordering::SeqCst) == 0).then(|| release.clone()),
                },
            )
        })
    };
    readiness
        .recv_timeout(Duration::from_secs(5))
        .expect("ready");
    let endpoint = lifecycle::read_endpoint(&cfg).expect("endpoint");
    let guard = std::thread::spawn(move || {
        let mut request = request("guard");
        request.timeout = Duration::from_secs(30);
        remote_execute(&endpoint, request)
    });
    calls
        .recv_timeout(Duration::from_secs(5))
        .expect("guard active");
    for expected in 1..=2 {
        let status = std::process::Command::new(std::env::current_exe().expect("executable"))
            .args(["--exact", "broker::tests::process_client", "--nocapture"])
            .env("ITDA_BROKER_TEST_STATE", &cfg.state_dir)
            .env("ITDA_BROKER_TEST_COUNT", expected.to_string())
            .status()
            .expect("client process");
        assert!(status.success());
        calls.recv_timeout(Duration::from_secs(5)).expect("call");
    }
    release.wait();
    guard.join().expect("guard join").expect("guard");
    BrowserClient::broker(cfg.clone())
        .shutdown()
        .expect("shutdown");
    owner_released(&cfg);
    worker.join().expect("join").expect("shutdown exit");
    assert!(!cfg.state_dir.join("endpoint.json").exists());
    assert!(cfg.state_dir.join("owner.lock").exists());
    fs::remove_dir_all(&cfg.state_dir).expect("cleanup");
}

#[test]
fn inactivity_closes_service_and_releases_owner() {
    let cfg = config();
    let (ready, _readiness) = mpsc::channel();
    let (entered, _calls) = mpsc::channel();
    run_with(
        cfg.clone(),
        ChannelWriter(ready),
        Duration::ZERO,
        move || Fixture {
            calls: 0,
            entered: entered.clone(),
            release: None,
        },
    )
    .expect("idle exit");
    owner_released(&cfg);
    assert!(!cfg.state_dir.join("endpoint.json").exists());
    fs::remove_dir_all(&cfg.state_dir).expect("cleanup");
}

#[test]
fn active_execution_prevents_idle_exit_until_cleanup_completes() {
    let service = Arc::new(Service {
        state: Mutex::new(Activity {
            active: 1,
            touched: Instant::now(),
            stopping: false,
            idle_waiting: false,
        }),
        changed: Condvar::new(),
    });
    let active = Active(service.clone());
    let timer = {
        let service = service.clone();
        std::thread::spawn(move || service.wait_until_idle(Duration::ZERO))
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut state = service.state.lock().expect("state");
    while !state.idle_waiting {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("idle waiter");
        (state, _) = service
            .changed
            .wait_timeout(state, left)
            .expect("idle wait");
    }
    assert!(!state.stopping);
    drop(state);
    drop(active);
    assert!(timer.join().expect("timer"));
}

#[test]
fn pool_shutdown_waits_for_active_cleanup_and_allows_local_reuse() {
    let (entered, calls) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let created = AtomicUsize::new(0);
    let pool = Arc::new(pool::Pool::new({
        let release = release.clone();
        move || Fixture {
            calls: 0,
            entered: entered.clone(),
            release: (created.fetch_add(1, Ordering::SeqCst) == 0).then(|| release.clone()),
        }
    }));
    let active = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("a")))
    };
    calls.recv_timeout(Duration::from_secs(5)).expect("active");
    let queued = {
        let pool = pool.clone();
        std::thread::spawn(move || pool.execute(request("a")))
    };
    pool.wait_for_waiters(1);
    let (ack, acknowledged) = mpsc::channel();
    let shutdown = {
        let pool = pool.clone();
        std::thread::spawn(move || {
            pool.shutdown(Duration::from_secs(5), false)
                .expect("shutdown");
            ack.send(()).expect("ack");
        })
    };
    assert_eq!(
        queued
            .join()
            .expect("join")
            .expect_err("invalidated")
            .completion,
        BrowserCompletion::NotStarted
    );
    assert!(acknowledged.try_recv().is_err());
    release.wait();
    active.join().expect("join").expect("active");
    shutdown.join().expect("join");
    assert_eq!(
        pool.execute(request("a")).expect("new local session").bytes,
        1_usize.to_be_bytes()
    );
}

struct ChannelWriter(mpsc::Sender<()>);

impl Write for ChannelWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.send(()).expect("ready receiver");
        Ok(())
    }
}
