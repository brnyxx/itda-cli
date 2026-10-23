use super::*;
use axum::{Router, response::Html, routing::get};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn reused_target_is_blank_between_actions_and_reset_discards_it() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/", get(|| async { Html("<h1>ready</h1>") })),
        )
        .await
        .unwrap();
    });
    tokio::task::spawn_blocking(move || {
        let mut transport = BrowserTransport::new();
        let request = BrowserRequest {
            url: format!("{origin}/"),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            account_id: "lifecycle".into(),
            credential_fingerprint: Some(String::new()),
            read_only: true,
            limit: 4096,
            timeout: Duration::from_secs(10),
            allow_local: true,
            mode: BrowserMode::Render,
            cooldowns: Vec::new(),
        };
        let mut sessions = Vec::new();
        for _ in 0..2 {
            transport.execute(request.clone()).unwrap();
            let browser = transport.browsers.front_mut().unwrap();
            let session = browser.page.as_ref().unwrap().session.clone();
            let deadline = Instant::now() + Duration::from_secs(2);
            let id = browser
                .send(Some(&session), "Target.getTargetInfo", json!({}), deadline)
                .unwrap();
            loop {
                let message = browser.read(deadline).unwrap();
                if message["id"].as_u64() == Some(id) {
                    assert_eq!(message["result"]["targetInfo"]["url"], "about:blank");
                    break;
                }
            }
            sessions.push(session);
        }
        assert_eq!(sessions[0], sessions[1]);
        transport.reset(&origin, "lifecycle");
        assert!(transport.browsers.is_empty());
        transport.execute(request).unwrap();
        assert_ne!(
            transport
                .browsers
                .front()
                .unwrap()
                .page
                .as_ref()
                .unwrap()
                .session,
            sessions[0],
        );
    })
    .await
    .unwrap();
    server.abort();
}
