use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::get,
};
use itda_browser::transport::{BrowserMode, BrowserRequest, BrowserTransport};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn request(url: String) -> BrowserRequest {
    BrowserRequest {
        url,
        method: "GET".into(),
        headers: Vec::new(),
        body: None,
        account_id: "cooldown".into(),
        credential_fingerprint: None,
        read_only: true,
        limit: 64 * 1024,
        // The first call starts the browser, which a busy CI runner can take
        // most of ten seconds to do.
        timeout: Duration::from_secs(30),
        allow_local: true,
        mode: BrowserMode::Render,
        cooldowns: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn optional_image_limit_preserves_document_and_blocks_later_requests() {
    optional_image_limit(StatusCode::TOO_MANY_REQUESTS, None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn optional_positive_retry_after_preserves_document_and_blocks_later_requests() {
    optional_image_limit(StatusCode::OK, Some("120")).await;
}

async fn optional_image_limit(status: StatusCode, retry_after: Option<&'static str>) {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let hits = Arc::new(AtomicUsize::new(0));
    let received = hits.clone();
    let cdn = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind CDN fixture");
    let cdn_origin = format!("http://{}", cdn.local_addr().expect("CDN address"));
    let cdn_server = tokio::spawn(async move {
        axum::serve(
            cdn,
            Router::new().route(
                "/optional.png",
                get(move || {
                    received.fetch_add(1, Ordering::SeqCst);
                    async move {
                        let mut headers = HeaderMap::new();
                        if let Some(value) = retry_after {
                            headers
                                .insert("retry-after", value.parse().expect("retry-after header"));
                        }
                        (status, headers, "limited")
                    }
                }),
            ),
        )
        .await
        .expect("serve CDN fixture")
    });
    let page = format!(
        r#"<html><body><h1>usable document</h1><img src="{cdn_origin}/optional.png?version=private" onerror="document.body.dataset.image='blocked'"></body></html>"#
    );
    let local = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind page fixture");
    let url = format!("http://{}/page", local.local_addr().expect("page address"));
    let server = tokio::spawn(async move {
        axum::serve(
            local,
            Router::new().route(
                "/page",
                get(move || {
                    let page = page.clone();
                    async { Html(page) }
                }),
            ),
        )
        .await
        .expect("serve page fixture")
    });
    tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let before = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("wall clock after epoch")
                .as_millis(),
        )
        .expect("millisecond timestamp fits");
        let mut limits = Vec::new();
        for attempt in 0..3 {
            let mut input = request(url.clone());
            if attempt == 2 {
                transport = BrowserTransport::new();
                input.cooldowns =
                    serde_json::from_slice(&serde_json::to_vec(&limits).expect("encode cooldowns"))
                        .expect("decode cooldowns");
            }
            let response = transport.execute(input).expect("browser recovery");
            assert_eq!(response.status, 200);
            assert_eq!(response.url, url);
            let html = String::from_utf8(response.bytes).expect("rendered UTF-8");
            assert!(html.contains("<h1>usable document</h1>"), "{html}");
            assert!(html.contains("data-image=\"blocked\""), "{html}");
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert_eq!(response.resource_limits.len(), 1);
            assert_eq!(response.resource_limits[0].origin, cdn_origin);
            if attempt == 0 {
                let after = i64::try_from(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("wall clock after epoch")
                        .as_millis(),
                )
                .expect("millisecond timestamp fits");
                let delay = if retry_after.is_some() {
                    120_000
                } else {
                    60_000
                };
                assert!(
                    (before + delay..=after + delay)
                        .contains(&response.resource_limits[0].until_ms)
                );
                limits = response.resource_limits;
            } else {
                assert_eq!(response.resource_limits[0].until_ms, limits[0].until_ms);
            }
        }
    })
    .await
    .expect("browser worker joined");
    server.abort();
    cdn_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn action_limits_remain_terminal_and_block_warm_and_seeded_calls() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let hits = Arc::new(AtomicUsize::new(0));
    let received = hits.clone();
    let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", local.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            local,
            Router::new().route(
                "/limited",
                get(move || {
                    received.fetch_add(1, Ordering::SeqCst);
                    async { (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "120")], "") }
                }),
            ),
        )
        .await
        .unwrap()
    });
    tokio::task::spawn_blocking(move || {
        for mode in [BrowserMode::Fetch, BrowserMode::Render] {
            let mut transport = BrowserTransport::new();
            let mut input = request(format!("{origin}/limited"));
            input.mode = mode;
            let before = hits.load(Ordering::SeqCst);
            let response = transport.execute(input.clone()).unwrap();
            assert_eq!(response.status, 429);
            assert_eq!(response.resource_limits.len(), 1);
            assert_eq!(response.resource_limits[0].origin, origin);
            let limits = response.resource_limits;
            for fresh in [false, true] {
                if fresh {
                    transport = BrowserTransport::new();
                    input.cooldowns = limits.clone();
                }
                let response = transport.execute(input.clone()).unwrap();
                assert_eq!(response.status, 429);
                assert!(response.bytes.is_empty());
                let remaining: u64 = response
                    .headers
                    .iter()
                    .find(|(name, _)| name == "retry-after")
                    .unwrap()
                    .1
                    .parse()
                    .unwrap();
                assert!((1..=120).contains(&remaining));
                assert_eq!(response.resource_limits[0].until_ms, limits[0].until_ms);
                assert_eq!(hits.load(Ordering::SeqCst), before + 1);
            }
        }
    })
    .await
    .unwrap();
    server.abort();
}
