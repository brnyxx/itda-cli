use axum::{
    Router,
    http::HeaderMap,
    response::Html,
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
        account_id: "driver".into(),
        credential_fingerprint: None,
        read_only: true,
        limit: 256 * 1024,
        // The first call starts the browser, which a busy CI runner can take
        // most of ten seconds to do.
        timeout: Duration::from_secs(30),
        allow_local: true,
        mode,
        cooldowns: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_inputs_preserve_rotated_cookie_seeds_and_storage() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let router = Router::new()
        .route("/seed", get(|| async {
            ([("set-cookie", "sid=rotated; Path=/; HttpOnly")], Html(
                "<script>localStorage.setItem('kept','yes')</script><h1>seeded</h1>"
            ))
        }))
        .route("/next", get(|headers: HeaderMap| async move {
            Html(format!(
                "<h1>{}</h1><script>document.body.dataset.kept=localStorage.getItem('kept')</script>",
                headers.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or("")
            ))
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let pages = tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let mut seed = request(format!("{origin}/seed?page=1"), BrowserMode::Render);
        seed.headers.push(("cookie".into(), "sid=seed".into()));
        transport.execute(seed).unwrap();
        [2, 3].map(|page| {
            let mut next = request(format!("{origin}/next?page={page}"), BrowserMode::Render);
            next.headers = vec![
                ("cookie".into(), "sid=seed".into()),
                ("accept".into(), format!("text/html; q=0.{page}")),
            ];
            String::from_utf8(transport.execute(next).unwrap().bytes).unwrap()
        })
    })
    .await
    .unwrap();
    server.abort();
    for page in pages {
        assert!(page.contains("<h1>sid=rotated</h1>"), "{page}");
        assert!(page.contains("data-kept=\"yes\""), "{page}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_scripts_frames_and_nested_workers_are_guarded() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let foreign = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let foreign_origin = format!("http://{}", foreign.local_addr().unwrap());
    let foreign_log = seen.clone();
    let foreign_router = Router::new().fallback(any(move |req: axum::extract::Request| {
        let seen = foreign_log.clone();
        async move {
            let path = req.uri().path().to_owned();
            seen.lock().unwrap().push((
                path.clone(), req.method().to_string(), req.headers().clone()
            ));
            let (mime, body) = match path.as_str() {
                "/script.js" => ("text/javascript", "document.body.dataset.script='yes';"),
                "/frame" => ("text/html", "<script>fetch('/frame-data').then(()=>parent.postMessage('frame-ready','*'))</script>"),
                "/worker-data" => ("text/plain", "worker-ready"),
                _ => ("text/plain", "ok"),
            };
            ([
                ("content-type", mime),
                ("access-control-allow-origin", "*"),
                ("set-cookie", "foreign=must-not-merge; Path=/"),
            ], body)
        }
    }));
    let foreign_server =
        tokio::spawn(async move { axum::serve(foreign, foreign_router).await.unwrap() });
    let page = format!(
        r#"<html><body><h1>Checking your browser</h1>
        <script>
        const done = new Set();
        addEventListener('message', e => {{
            if (e.data === 'frame-ready') done.add('frame');
            finish();
        }});
        function finish() {{
            if (done.size === 2 && document.body.dataset.script === 'yes')
                document.querySelector('h1').textContent='resources completed';
        }}
        const w = new Worker('/worker.js');
        w.onmessage = e => {{ if(e.data === 'worker-ready') done.add('worker'); finish(); }};
        fetch('{foreign_origin}/delete').catch(()=>{{}});
        fetch('{foreign_origin}/write',{{method:'POST',body:'bad'}}).catch(()=>{{}});
        </script>
        <script src="{foreign_origin}/script.js"></script>
        <iframe src="{foreign_origin}/frame"></iframe>
        </body></html>"#
    );
    let nested = format!(
        "fetch('{foreign_origin}/worker-data').then(r=>r.text()).then(x=>postMessage(x)); fetch('{foreign_origin}/worker-write',{{method:'POST',body:'bad'}}).catch(()=>{{}});"
    );
    let router = Router::new()
        .route(
            "/page",
            get(move || {
                let page = page.clone();
                async { Html(page) }
            }),
        )
        .route(
            "/worker.js",
            get(|| async {
                (
                    [("content-type", "text/javascript")],
                    "const child=new Worker('/nested.js'); child.onmessage=e=>postMessage(e.data);",
                )
            }),
        )
        .route(
            "/nested.js",
            get(move || {
                let nested = nested.clone();
                async { ([("content-type", "text/javascript")], nested) }
            }),
        )
        .route(
            "/cookies",
            get(|headers: HeaderMap| async move {
                headers
                    .get("cookie")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_owned()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let response = tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let mut input = request(
            format!("{origin}/page?token=action-secret"),
            BrowserMode::Render,
        );
        input.headers = vec![
            ("authorization".into(), "Bearer action-secret".into()),
            ("cookie".into(), "sid=action-secret".into()),
            ("x-api-key".into(), "action-secret".into()),
        ];
        transport.execute(input.clone()).inspect(|_| {
            input.url = format!("{origin}/cookies?token=action-secret");
            input.mode = BrowserMode::Fetch;
            let cookies = String::from_utf8(transport.execute(input).unwrap().bytes).unwrap();
            assert!(cookies.contains("sid=action-secret"), "{cookies}");
            assert!(!cookies.contains("foreign="), "{cookies}");
        })
    })
    .await
    .unwrap()
    .unwrap_or_else(|error| panic!("{error:?}; requests: {:?}", seen.lock().unwrap()));
    server.abort();
    foreign_server.abort();
    let html = String::from_utf8(response.bytes).unwrap();
    assert!(html.contains("<h1>resources completed</h1>"), "{html}");
    let seen = seen.lock().unwrap();
    for path in ["/script.js", "/frame", "/frame-data", "/worker-data"] {
        assert!(seen.iter().any(|(p, _, _)| p == path), "{path}: {seen:?}");
    }
    for (path, method, headers) in &*seen {
        assert_eq!(method, "GET", "{path}");
        assert!(!path.contains("write") && path != "/delete", "{path}");
        for key in ["cookie", "authorization", "x-api-key", "referer"] {
            assert!(
                headers
                    .get(key)
                    .is_none_or(|value| value.as_bytes().is_empty()),
                "{path} leaked {key}: {headers:?}"
            );
        }
    }
    assert!(
        !response
            .headers
            .iter()
            .any(|(_, v)| v.contains("foreign=must-not-merge"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn classified_post_is_sent_once_and_never_replayed_on_redirect() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = count.clone();
    let router = Router::new().route(
        "/query",
        any(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async {
                (
                    axum::http::StatusCode::TEMPORARY_REDIRECT,
                    [("location", "/query")],
                    "",
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let mut input = request(format!("{origin}/query"), BrowserMode::Fetch);
        input.method = "POST".into();
        input.body = Some(b"query".to_vec());
        input.read_only = false;
        assert_eq!(
            transport.execute(input.clone()).unwrap_err().public_code,
            "unsafe_method"
        );
        input.read_only = true;
        assert_eq!(
            transport.execute(input).unwrap_err().public_code,
            "unsafe_target"
        );
    })
    .await
    .unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn quiescence_blocks_pagehide_egress_and_keeps_storage() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let boundary_count = count.clone();
    let seen = count.clone();
    let router = Router::new()
        .route("/page", get(|| async { Html(
            "<html><body>ready<script>
            localStorage.setItem('alive','yes');
            addEventListener('pagehide',()=>fetch('/idle-hit',{keepalive:true}));
            </script></body></html>"
        ) }))
        .route("/idle-hit", get(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { "unwanted" }
        }))
        .route("/after", get(|| async { Html(
            "<html><body><script>document.body.dataset.alive=localStorage.getItem('alive')</script></body></html>"
        ) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        transport
            .execute(request(format!("{origin}/page"), BrowserMode::Render))
            .unwrap();
        assert_eq!(
            boundary_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "first action boundary"
        );
        let page = transport
            .execute(request(format!("{origin}/after"), BrowserMode::Render))
            .unwrap();
        let html = String::from_utf8(page.bytes).unwrap();
        assert!(html.contains("data-alive=\"yes\""), "{html}");
    })
    .await
    .unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reused_target_does_not_retain_per_call_header_overrides() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/headers",
                get(|headers: HeaderMap| async move {
                    axum::Json(serde_json::json!({
                        "agent":headers.get("user-agent").and_then(|v|v.to_str().ok()),
                        "authorization":headers.get("authorization").and_then(|v|v.to_str().ok()),
                    }))
                }),
            ),
        )
        .await
        .unwrap()
    });
    tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let mut first = request(format!("{origin}/headers"), BrowserMode::Fetch);
        first.credential_fingerprint = Some("stable".into());
        first.headers = vec![
            ("user-agent".into(), "itda-first-agent".into()),
            ("authorization".into(), "Bearer first".into()),
        ];
        let response = transport.execute(first.clone()).unwrap();
        let first_body: serde_json::Value = serde_json::from_slice(&response.bytes).unwrap();
        assert_eq!(first_body["agent"], "itda-first-agent");
        assert_eq!(first_body["authorization"], "Bearer first");
        first.headers.clear();
        let response = transport.execute(first).unwrap();
        let second_body: serde_json::Value = serde_json::from_slice(&response.bytes).unwrap();
        assert!(second_body["agent"].as_str().unwrap().contains("Chrome/"));
        assert!(second_body["authorization"].is_null());
    })
    .await
    .unwrap();
    server.abort();
}
