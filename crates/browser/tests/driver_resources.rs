use axum::{
    Router,
    extract::Request,
    response::Html,
    routing::{any, get},
};
use itda_browser::transport::{BrowserMode, BrowserRequest, BrowserTransport};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[tokio::test(flavor = "multi_thread")]
async fn oopif_shared_and_service_workers_have_guarded_egress() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let foreign = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = format!("http://localhost:{}", foreign.local_addr().unwrap().port());
    let remote_log = seen.clone();
    let foreign_server = tokio::spawn(async move {
        axum::serve(foreign, Router::new().fallback(any(move |request: Request| {
            let log = remote_log.clone();
            async move {
                let path = request.uri().path().to_owned();
                log.lock().unwrap().push((path.clone(), request.method().to_string(), request.headers().clone()));
                let body = if path == "/frame" {
                    "<script>fetch('/frame-data').then(()=>parent.postMessage('frame-ready','*'))</script>"
                } else { "data" };
                ([("access-control-allow-origin","*"), ("content-type","text/html")], body)
            }
        }))).await.unwrap()
    });
    let page = format!(
        r#"<html><body><h1>Checking your browser</h1><script>
        const done=new Set();
        function ready(value) {{
            done.add(value);
            if(done.has('frame-ready')&&done.has('shared-ready')&&done.has('service-ready'))
                document.querySelector('h1').textContent='all resources ready';
        }}
        addEventListener('message',e=>ready(e.data));
        const shared=new SharedWorker('/shared.js');
        shared.port.onmessage=e=>ready(e.data);
        shared.port.start();
        navigator.serviceWorker.addEventListener('message',e=>ready(e.data));
        navigator.serviceWorker.register('/service.js');
        </script><iframe src="{remote}/frame"></iframe></body></html>"#
    );
    let shared = format!(
        "onconnect=e=>{{const p=e.ports[0];fetch('{remote}/shared-data').then(()=>p.postMessage('shared-ready'));fetch('{remote}/shared-write',{{method:'POST',body:'bad'}}).catch(()=>{{}})}};"
    );
    let service = format!(
        "addEventListener('install',()=>skipWaiting());addEventListener('activate',e=>e.waitUntil((async()=>{{await clients.claim();await fetch('{remote}/service-data');for(const c of await clients.matchAll({{includeUncontrolled:true}}))c.postMessage('service-ready');fetch('{remote}/service-write',{{method:'POST',body:'bad'}}).catch(()=>{{}})}})()));"
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
            "/shared.js",
            get(move || {
                let shared = shared.clone();
                async { ([("content-type", "text/javascript")], shared) }
            }),
        )
        .route(
            "/service.js",
            get(move || {
                let service = service.clone();
                async { ([("content-type", "text/javascript")], service) }
            }),
        );
    let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", local.local_addr().unwrap());
    let local_server = tokio::spawn(async move { axum::serve(local, router).await.unwrap() });
    let result = tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        transport.execute(BrowserRequest {
            url: format!("{origin}/page"),
            method: "GET".into(),
            headers: vec![
                ("authorization".into(), "Bearer isolated".into()),
                ("cookie".into(), "sid=isolated".into()),
            ],
            body: None,
            account_id: "resources".into(),
            credential_fingerprint: Some("first".into()),
            read_only: true,
            limit: 256 * 1024,
            // The first call starts the browser, which a busy CI runner can take
            // most of ten seconds to do.
            timeout: Duration::from_secs(30),
            allow_local: true,
            mode: BrowserMode::Render,
            cooldowns: Vec::new(),
        })
    })
    .await
    .unwrap();
    let result =
        result.unwrap_or_else(|error| panic!("{error:?}; requests: {:?}", seen.lock().unwrap()));
    let html = String::from_utf8(result.bytes).unwrap();
    assert!(
        html.contains("<h1>all resources ready</h1>"),
        "{html}; requests: {:?}",
        seen.lock().unwrap()
    );
    let seen = seen.lock().unwrap();
    for path in ["/frame", "/frame-data", "/shared-data", "/service-data"] {
        assert!(seen.iter().any(|(p, _, _)| p == path), "{seen:?}");
    }
    for (path, method, headers) in &*seen {
        assert_eq!(method, "GET", "{path}");
        assert!(!path.contains("write"), "{path}");
        for key in ["authorization", "cookie", "referer"] {
            assert!(
                headers.get(key).is_none_or(|v| v.as_bytes().is_empty()),
                "{path}: {headers:?}"
            );
        }
    }
    local_server.abort();
    foreign_server.abort();
}
