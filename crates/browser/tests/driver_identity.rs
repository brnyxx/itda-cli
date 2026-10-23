//! The browser transport presents the same user agent as direct HTTP and
//! keeps the browser's own client hints. Runs only with `ITDA_TEST_BROWSER`
//! set to an installed browser.
use axum::{Router, http::HeaderMap, response::Html, routing::get};
use itda_browser::transport::{BrowserMode, BrowserRequest, BrowserTransport};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[tokio::test(flavor = "multi_thread")]
async fn the_browser_looks_like_direct_http_and_keeps_its_client_hints() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let direct = itda_browser::transport::user_agent().expect("direct-HTTP user agent");
    assert!(!direct.contains("Headless"), "{direct}");
    let seen = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let log = seen.clone();
    let router = Router::new().route(
        "/",
        get(move |headers: HeaderMap| {
            let log = log.clone();
            async move {
                let header = |name: &str| {
                    headers
                        .get(name)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned()
                };
                log.lock()
                    .unwrap()
                    .push((header("user-agent"), header("sec-ch-ua")));
                Html(
                    "<html><body><h1 id=x></h1><script>document.getElementById('x').textContent=\
                     navigator.userAgent+'|'+navigator.userAgentData.brands.length</script></body></html>",
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let pages = tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        [BrowserMode::Fetch, BrowserMode::Render].map(|mode| {
            let response = transport
                .execute(BrowserRequest {
                    url: format!("{origin}/"),
                    method: "GET".into(),
                    headers: Vec::new(),
                    body: None,
                    account_id: "identity".into(),
                    credential_fingerprint: None,
                    read_only: true,
                    limit: 64 * 1024,
                    timeout: Duration::from_secs(30),
                    allow_local: true,
                    mode,
                    cooldowns: Vec::new(),
                })
                .unwrap();
            String::from_utf8(response.bytes).unwrap()
        })
    })
    .await
    .unwrap();
    for (agent, hints) in seen.lock().unwrap().iter() {
        assert_eq!(agent, &direct);
        assert!(!hints.is_empty(), "no client hints with {agent}");
        assert!(!hints.contains("Headless"), "{hints}");
    }
    assert!(seen.lock().unwrap().len() >= 2);
    let rendered = &pages[1];
    assert!(rendered.contains(&format!("{direct}|")), "{rendered}");
    assert!(!rendered.contains(&format!("{direct}|0")), "{rendered}");
    server.abort();
}
