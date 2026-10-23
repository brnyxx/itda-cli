//! How large an answer the browser transport carries. Runs only with
//! `ITDA_TEST_BROWSER` set to an installed browser.
use axum::{Router, extract::Path, response::Html, routing::get};
use itda_browser::transport::{BrowserMode, BrowserRequest, BrowserTransport};
use std::time::Duration;

const MIB: usize = 1024 * 1024;

fn request(url: String, mode: BrowserMode) -> BrowserRequest {
    BrowserRequest {
        url,
        method: "GET".into(),
        headers: Vec::new(),
        body: None,
        account_id: "sizes".into(),
        credential_fingerprint: None,
        read_only: true,
        limit: 16 * MIB,
        timeout: Duration::from_secs(120),
        allow_local: true,
        mode,
        cooldowns: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bodies_up_to_the_cap_arrive_whole() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let router = Router::new()
        .route(
            "/bytes/{mib}",
            get(|Path(mib): Path<usize>| async move { vec![b'x'; mib * MIB] }),
        )
        .route(
            "/page/{mib}",
            get(|Path(mib): Path<usize>| async move {
                Html(format!(
                    "<html><body>{}</body></html>",
                    "y".repeat(mib * MIB)
                ))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        for mib in [1usize, 8, 15] {
            let started = std::time::Instant::now();
            let fetched =
                transport.execute(request(format!("{origin}/bytes/{mib}"), BrowserMode::Fetch));
            eprintln!(
                "fetch {mib} MiB: {:?} in {:?}",
                fetched
                    .as_ref()
                    .map(|r| r.bytes.len())
                    .map_err(|e| e.public_code.clone()),
                started.elapsed()
            );
            let started = std::time::Instant::now();
            let rendered =
                transport.execute(request(format!("{origin}/page/{mib}"), BrowserMode::Render));
            eprintln!(
                "render {mib} MiB: {:?} in {:?}",
                rendered
                    .as_ref()
                    .map(|r| r.bytes.len())
                    .map_err(|e| e.public_code.clone()),
                started.elapsed()
            );
            let fetched = fetched.unwrap();
            assert_eq!(fetched.bytes.len(), mib * MIB);
            assert!(fetched.bytes.iter().all(|b| *b == b'x'));
            let rendered = rendered.unwrap();
            assert!(rendered.bytes.len() >= mib * MIB);
        }
    })
    .await
    .unwrap();
    server.abort();
}
