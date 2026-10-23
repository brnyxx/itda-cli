use super::*;
use crate::adaptive::Browser;
use axum::{Router, http::StatusCode, response::IntoResponse, routing::any};
use itda_browser::transport::{BrowserMode, BrowserRequest, BrowserResponse};
use itda_store::Store;
use std::sync::{
    Mutex,
    atomic::{AtomicI64, Ordering},
};

type Events = Arc<Mutex<Vec<String>>>;

async fn host(events: Events, status: StatusCode, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().fallback(any(move || {
        events.lock().unwrap().push("direct".into());
        async move { (status, body).into_response() }
    }));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    address
}

fn network(
    store: Store,
    clock: Arc<AtomicI64>,
    events: Events,
    answer: impl FnMut(BrowserRequest) -> AppResult<BrowserResponse> + Send + 'static,
) -> Network {
    let gate = Gate::with_limits(store, 0, 1000)
        .unwrap()
        .with_clock(move || clock.load(Ordering::SeqCst));
    let mut net = Network::adaptive(Arc::new(gate), true);
    let mut answer = answer;
    net.browser = Some(Arc::new(Browser::Test(std::sync::Mutex::new(Box::new(
        move |request| {
            events.lock().unwrap().push(
                match request.mode {
                    BrowserMode::Fetch => "fetch",
                    BrowserMode::Render => "render",
                }
                .into(),
            );
            answer(request)
        },
    )))));
    net
}

fn reply(request: BrowserRequest, status: u16, body: &str) -> BrowserResponse {
    BrowserResponse {
        url: request.url,
        status,
        headers: vec![("content-type".into(), "text/html".into())],
        bytes: body.as_bytes().to_vec(),
        resource_limits: Vec::new(),
    }
}

fn clock() -> Arc<AtomicI64> {
    Arc::new(AtomicI64::new(1_700_000_000_000))
}

#[tokio::test]
async fn optional_resource_limits_persist_without_throttling_the_document() {
    let events = Events::default();
    let base = host(
        events.clone(),
        StatusCode::FORBIDDEN,
        "<title>Just a moment</title>",
    )
    .await;
    let store = Store::in_memory().unwrap();
    let clock = clock();
    let until = clock.load(Ordering::SeqCst) + 60_000;
    for attempt in 0..2 {
        let net = network(
            store.clone(),
            clock.clone(),
            events.clone(),
            move |request| {
                assert_eq!(request.cooldowns.len(), attempt);
                let mut response = reply(request, 200, "verified");
                response
                    .resource_limits
                    .push(itda_browser::transport::OriginCooldown {
                        origin: "https://cdn.example.test".into(),
                        until_ms: until,
                    });
                Ok(response)
            },
        );
        let response = net.fetch(Request::read(&base)).await.unwrap();
        assert!(response.successful());
        assert_eq!(
            net.gate
                .admit_origin("https://cdn.example.test", None)
                .await
                .unwrap_err()
                .public_code,
            "rate_limited"
        );
    }
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch", "fetch"]);
}

#[tokio::test]
async fn missing_data_during_resource_cooldown_does_not_poison_a_strategy() {
    let events = Events::default();
    let base = host(
        events.clone(),
        StatusCode::FORBIDDEN,
        "<title>Just a moment</title>",
    )
    .await;
    let clock = clock();
    let until = clock.load(Ordering::SeqCst) + 60_000;
    let mut attempts = 0;
    let net = network(
        Store::in_memory().unwrap(),
        clock,
        events.clone(),
        move |request| {
            assert_eq!(request.mode, BrowserMode::Fetch);
            attempts += 1;
            let mut response = reply(
                request,
                200,
                if attempts == 1 { "loading" } else { "verified" },
            );
            response
                .resource_limits
                .push(itda_browser::transport::OriginCooldown {
                    origin: "https://cdn.example.test".into(),
                    until_ms: until,
                });
            Ok(response)
        },
    );
    let inspect = |response: &Response| {
        if response.bytes == b"verified" {
            ResponseVerdict::Valid
        } else {
            ResponseVerdict::Recover
        }
    };
    let error = net
        .fetch_validated(Request::read(&base), "article", inspect)
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "rate_limited");
    assert_eq!(error.extra["retry_after"], 60);
    assert_eq!(error.extra["upstream_origin"], "https://cdn.example.test");
    assert!(
        net.fetch_validated(Request::read(&base), "article", inspect)
            .await
            .unwrap()
            .successful()
    );
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch", "fetch"]);
}

#[tokio::test]
async fn recovery_continues_until_the_declared_response_is_valid() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::OK, "<div id=\"app\"></div>").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| {
            let body = match request.mode {
                BrowserMode::Fetch => "<div id=\"app\"></div>",
                BrowserMode::Render => "<h1>verified data</h1>",
            };
            Ok(reply(request, 200, body))
        },
    );
    for _ in 0..2 {
        let response = net
            .fetch_validated(Request::read(&base), "article-v1", |response| {
                if response.bytes == b"<h1>verified data</h1>" {
                    ResponseVerdict::Valid
                } else {
                    ResponseVerdict::Recover
                }
            })
            .await
            .unwrap();
        assert_eq!(response.bytes, b"<h1>verified data</h1>");
    }
    assert_eq!(
        *events.lock().unwrap(),
        ["direct", "fetch", "render", "render"]
    );
}

#[tokio::test]
async fn terminal_application_errors_do_not_retry_browser_transports() {
    let events = Events::default();
    let base = host(
        events.clone(),
        StatusCode::OK,
        "{\"errors\":[\"invalid query\"]}",
    )
    .await;
    let net = network(Store::in_memory().unwrap(), clock(), events.clone(), |_| {
        panic!("application error must not replay the request")
    });
    let response = net
        .fetch_validated(Request::read(base), "query-v1", |_| {
            ResponseVerdict::Terminal
        })
        .await
        .unwrap();
    assert_eq!(response.json().unwrap()["errors"][0], "invalid query");
    assert_eq!(*events.lock().unwrap(), ["direct"]);
}

#[tokio::test]
async fn response_contracts_do_not_share_failed_route_records() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::OK, "direct data").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| Ok(reply(request, 200, "browser data")),
    );
    net.fetch_validated(Request::read(&base), "browser-contract", |response| {
        if response.bytes == b"browser data" {
            ResponseVerdict::Valid
        } else {
            ResponseVerdict::Recover
        }
    })
    .await
    .unwrap();
    let direct = net
        .fetch_validated(Request::read(&base), "direct-contract", |_| {
            ResponseVerdict::Valid
        })
        .await
        .unwrap();
    assert_eq!(direct.bytes, b"direct data");
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch", "direct"]);
}

#[tokio::test]
async fn a_validated_route_is_reused_when_only_query_inputs_change() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| Ok(reply(request, 200, "verified")),
    );
    for query in ["one", "two"] {
        net.fetch_validated(
            Request::read(format!("{base}/items?q={query}")),
            "items-v1",
            |_| ResponseVerdict::Valid,
        )
        .await
        .unwrap();
    }
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch", "fetch"]);
}

#[tokio::test]
async fn direct_success_does_not_open_a_browser() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::OK, "welcome").await;
    let net = network(Store::in_memory().unwrap(), clock(), events.clone(), |_| {
        panic!("browser must not open")
    });
    assert_eq!(
        net.fetch(Request::read(base)).await.unwrap().bytes,
        b"welcome"
    );
    assert_eq!(*events.lock().unwrap(), ["direct"]);
}

#[tokio::test]
async fn challenge_tries_fetch_then_render_without_intermediate_pause() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| {
            let (status, body) = match request.mode {
                BrowserMode::Fetch => (403, "Just a moment..."),
                BrowserMode::Render => (200, "rendered"),
            };
            Ok(reply(request, status, body))
        },
    );
    let response = net.fetch(Request::read(&base)).await.unwrap();
    assert_eq!(response.bytes, b"rendered");
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch", "render"]);
    assert_eq!(net.gate.paused(&format!("{base}|*")).await.unwrap(), None);
}

#[tokio::test]
async fn strategy_survives_network_recreation_and_is_account_scoped() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let store = Store::in_memory().unwrap();
    let time = clock();
    let make = || {
        network(store.clone(), time.clone(), events.clone(), |request| {
            Ok(reply(request, 200, "browser"))
        })
    };
    let request = |account: &str| Request {
        account_id: account.into(),
        ..Request::read(&base)
    };
    make().fetch(request("alice")).await.unwrap();
    make().fetch(request("alice")).await.unwrap();
    make().fetch(request("bob")).await.unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        ["direct", "fetch", "fetch", "direct", "fetch"]
    );
}

#[tokio::test]
async fn expired_preference_retries_direct_and_resume_clears_preference() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let time = clock();
    let net = network(
        Store::in_memory().unwrap(),
        time.clone(),
        events.clone(),
        |request| Ok(reply(request, 200, "browser")),
    );
    net.fetch(Request::read(&base)).await.unwrap();
    time.fetch_add(3_600_001, Ordering::SeqCst);
    net.fetch(Request::read(&base)).await.unwrap();
    net.resume(&base, "").await.unwrap();
    net.fetch(Request::read(&base)).await.unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        ["direct", "fetch", "direct", "fetch", "direct", "fetch"]
    );
}

#[tokio::test]
async fn read_only_post_never_renders_and_writes_never_replay() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| {
            assert_eq!(request.method, "POST");
            assert!(matches!(request.mode, BrowserMode::Fetch));
            Ok(reply(request, 403, "Just a moment..."))
        },
    );
    let request = Request {
        method: "POST".into(),
        body: Some(b"query".to_vec()),
        ..Request::read(&base)
    };
    assert_eq!(net.fetch(request).await.unwrap().defense, "cloudflare");
    net.resume(&base, "").await.unwrap();
    let write = Request {
        method: "POST".into(),
        read_only: false,
        ..Request::read(&base)
    };
    net.fetch(write).await.unwrap();
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch", "direct"]);
}

#[tokio::test]
async fn rate_limit_and_api_denial_never_escalate() {
    for (status, body) in [
        (StatusCode::TOO_MANY_REQUESTS, "slow down"),
        (StatusCode::UNAUTHORIZED, "sign in"),
        (StatusCode::FORBIDDEN, "{\"error\":\"forbidden\"}"),
    ] {
        let events = Events::default();
        let base = host(events.clone(), status, body).await;
        let net = network(Store::in_memory().unwrap(), clock(), events.clone(), |_| {
            panic!("denial must not escalate")
        });
        assert_eq!(
            net.fetch(Request::read(base)).await.unwrap().status,
            status.as_u16()
        );
        assert_eq!(*events.lock().unwrap(), ["direct"]);
    }
}

#[tokio::test]
async fn final_challenge_caches_only_the_request_without_pausing_the_site() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| Ok(reply(request, 403, "Just a moment...")),
    );
    assert_eq!(
        net.fetch(Request::read(&base)).await.unwrap().defense,
        "cloudflare"
    );
    assert_eq!(
        net.fetch(Request::read(&base))
            .await
            .unwrap_err()
            .public_code,
        gate::BLOCKED
    );
    assert_eq!(net.gate.paused(&format!("{base}|*")).await.unwrap(), None);
    net.fetch(Request::read(format!("{base}/other")))
        .await
        .unwrap();
    net.fetch(Request {
        account_id: "other".into(),
        ..Request::read(&base)
    })
    .await
    .unwrap();
    net.resume(&base, "").await.unwrap();
    net.fetch(Request::read(&base)).await.unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        [
            "direct", "fetch", "render", "direct", "fetch", "render", "direct", "fetch", "render",
            "direct", "fetch", "render",
        ]
    );
}

#[tokio::test]
async fn cached_failures_do_not_pause_other_queries_or_bodies() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| Ok(reply(request, 403, "Just a moment...")),
    );
    let request = |query: &str, body: &[u8]| Request {
        pause_on_block: false,
        method: "POST".into(),
        body: Some(body.to_vec()),
        ..Request::read(format!("{base}/api?q={query}"))
    };
    net.fetch(request("one", b"a")).await.unwrap();
    assert_eq!(
        net.fetch(request("one", b"a"))
            .await
            .unwrap_err()
            .public_code,
        gate::BLOCKED
    );
    net.fetch(request("two", b"a")).await.unwrap();
    net.fetch(request("one", b"b")).await.unwrap();
    assert_eq!(net.gate.paused(&format!("{base}|*")).await.unwrap(), None);
    assert_eq!(
        *events.lock().unwrap(),
        ["direct", "fetch", "direct", "fetch", "direct", "fetch"]
    );
}

#[tokio::test]
async fn browser_response_consumes_the_remaining_byte_budget() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let net = network(Store::in_memory().unwrap(), clock(), events, |request| {
        assert_eq!(request.limit, 4);
        Ok(reply(request, 200, "oversized"))
    });
    let input = Request {
        limit: 20,
        ..Request::read(base)
    };
    assert_eq!(net.fetch(input).await.unwrap_err().public_code, "too_large");
}

#[tokio::test]
async fn browser_attempts_consume_the_shared_gate_budget() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let mut net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| Ok(reply(request, 403, "Just a moment...")),
    );
    net.gate = Arc::new(Gate::with_limits(Store::in_memory().unwrap(), 0, 2).unwrap());
    assert_eq!(
        net.fetch(Request::read(base))
            .await
            .unwrap_err()
            .public_code,
        "rate_limited"
    );
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch"]);
}

#[tokio::test]
async fn failed_browser_fetch_falls_through_to_render() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| match request.mode {
            BrowserMode::Fetch => Err(AppError::api("request_failed", 502)),
            BrowserMode::Render => Ok(reply(request, 200, "rendered")),
        },
    );
    assert_eq!(
        net.fetch(Request::read(base)).await.unwrap().bytes,
        b"rendered"
    );
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch", "render"]);
}

#[tokio::test]
async fn cancellation_keeps_the_lease_until_browser_io_finishes() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let store = Store::in_memory().unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let mut started_tx = Some(started_tx);
    let net = Arc::new(network(store.clone(), clock(), events, move |request| {
        started_tx.take().unwrap().send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(reply(request, 200, "done"))
    }));
    let running = {
        let net = net.clone();
        let base = base.clone();
        tokio::spawn(async move { net.fetch(Request::read(base)).await })
    };
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    let scope = format!("{base}|");
    let leased: bool = store
        .call(move |conn| {
            Ok(conn.query_row(
                "SELECT lease != '' FROM request_gate WHERE scope=?1",
                [scope],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    release_tx.send(()).unwrap();
    assert!(
        leased,
        "caller cancellation released the gate while browser I/O still ran"
    );
}

#[tokio::test]
async fn adaptive_mode_recovers_a_historical_challenge_pause() {
    let events = Events::default();
    let base = host(events.clone(), StatusCode::FORBIDDEN, "Just a moment...").await;
    let store = Store::in_memory().unwrap();
    let time = clock();
    let strict_clock = time.clone();
    let strict = Network::new(
        Arc::new(
            Gate::with_limits(store.clone(), 0, 100)
                .unwrap()
                .with_clock(move || strict_clock.load(Ordering::SeqCst)),
        ),
        true,
    );
    strict.fetch(Request::read(&base)).await.unwrap();
    assert_eq!(
        strict
            .gate
            .paused(&format!("{base}|*"))
            .await
            .unwrap()
            .as_deref(),
        Some(gate::BLOCKED)
    );
    let adaptive = network(store, time, events, |request| {
        Ok(reply(request, 200, "recovered"))
    });
    assert_eq!(
        adaptive.fetch(Request::read(base)).await.unwrap().bytes,
        b"recovered"
    );
}

#[tokio::test]
async fn a_lost_worker_gets_the_read_one_more_try_and_then_the_next_way() {
    let events = Events::default();
    let base = host(
        events.clone(),
        StatusCode::FORBIDDEN,
        "<title>Just a moment</title>",
    )
    .await;
    let net = network(Store::in_memory().unwrap(), clock(), events.clone(), |_| {
        Err(AppError::api("browser_unavailable", 502).with("broker_gone", true))
    });
    // With every way exhausted the caller gets the challenge page itself,
    // never a cached body or a pause.
    let response = net.fetch(Request::read(&base)).await.unwrap();
    assert_eq!(response.status, 403);
    // The fetch is tried again once for the lost worker; the render that
    // follows is not, since the worker was already started again.
    assert_eq!(
        *events.lock().unwrap(),
        ["direct", "fetch", "fetch", "render"]
    );
}

#[tokio::test]
async fn an_unknown_outcome_ends_the_read_at_once() {
    let events = Events::default();
    let base = host(
        events.clone(),
        StatusCode::FORBIDDEN,
        "<title>Just a moment</title>",
    )
    .await;
    let net = network(Store::in_memory().unwrap(), clock(), events.clone(), |_| {
        Err(AppError::api("request_failed", 502).with("completion", "unknown"))
    });
    let error = net.fetch(Request::read(&base)).await.unwrap_err();
    assert_eq!(error.public_code, "request_failed");
    assert_eq!(error.extra["completion"], "unknown");
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch"]);
}

#[tokio::test]
async fn a_signed_in_401_from_the_browser_pauses_the_account_not_the_transport() {
    let events = Events::default();
    let base = host(
        events.clone(),
        StatusCode::FORBIDDEN,
        "<title>Just a moment</title>",
    )
    .await;
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        events.clone(),
        |request| Ok(reply(request, 401, "who are you")),
    );
    let request = || {
        let mut request = Request::read(&base);
        request.account_id = "alice".into();
        request
    };
    let response = net.fetch(request()).await.unwrap();
    assert_eq!(response.status, 401);
    assert_eq!(*events.lock().unwrap(), ["direct", "fetch"]);
    let origin = validate_url(&base).unwrap().origin().ascii_serialization();
    assert_eq!(
        net.gate.paused(&format!("{origin}|alice")).await.unwrap(),
        Some(gate::AUTH_REQUIRED.to_owned())
    );
    assert_eq!(net.gate.paused(&format!("{origin}|*")).await.unwrap(), None);
    assert_eq!(
        net.fetch(request()).await.unwrap_err().public_code,
        "auth_required"
    );
}

#[tokio::test]
async fn a_change_carries_the_same_user_agent_as_a_read() {
    let agents = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = agents.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().fallback(any(move |headers: axum::http::HeaderMap| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(
                headers
                    .get("user-agent")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned(),
            );
            "ok"
        }
    }));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let net = network(
        Store::in_memory().unwrap(),
        clock(),
        Events::default(),
        |_| Err(AppError::api("browser_unavailable", 502)),
    );
    net.fetch(Request::read(&base)).await.unwrap();
    let mut change = Request::read(&base);
    change.method = "POST".into();
    change.read_only = false;
    change.body = Some(b"title=hello".to_vec());
    net.fetch(change).await.unwrap();
    // The installed browser's user agent when there is one, else itda's own.
    let expected = itda_browser::transport::user_agent().unwrap_or(USER_AGENT.to_owned());
    assert_eq!(*agents.lock().unwrap(), [expected.clone(), expected]);
}
