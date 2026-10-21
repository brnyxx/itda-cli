//! Records a real browser using a local page. Runs only when
//! `ITDA_TEST_BROWSER` names a Chrome or Chromium binary, which is started
//! headless through a small wrapper script.
#![allow(clippy::unwrap_used)]
#![cfg(unix)]
use axum::{
    Router,
    response::Html,
    routing::{get, post},
};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test(flavor = "multi_thread")]
async fn records_the_calls_a_page_makes_with_their_bodies() {
    let Some(binary) = std::env::var_os("ITDA_TEST_BROWSER").filter(|b| !b.is_empty()) else {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    };
    let router = Router::new()
        .route(
            "/board",
            get(|| async {
                Html(
                    "<h1>Board</h1><img src='/logo.png'><script>
                     fetch('/api/items?q=x', {headers: {'X-Api-Key': 'recorded-key'}})
                       .then(r => r.json())
                       .then(() => fetch('/api/items', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({title: 'hi'})}));
                     </script>",
                )
            }),
        )
        .route(
            "/api/items",
            get(|| async { axum::Json(json!({"items": [{"title": "first"}], "total": 1})) })
                .post(|| async { (axum::http::StatusCode::CREATED, axum::Json(json!({"id": 2}))) }),
        )
        .route("/logo.png", get(|| async { "not really a png" }))
        .route("/other", post(|| async { "no" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let dir = std::env::temp_dir().join(format!("itda-record-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wrapper = dir.join("browser");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' --headless=new --no-sandbox --no-proxy-server \"$@\"\n",
            binary.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let page = url::Url::parse(&format!("{base}/board")).unwrap();
    let har = tokio::task::spawn_blocking(move || {
        let mut recorder = itda_browser::Recorder::open(&wrapper, &page).unwrap();
        for _ in 0..60 {
            recorder.poll(Duration::from_millis(250)).unwrap();
            if recorder.len() >= 3 {
                break;
            }
        }
        recorder.poll(Duration::from_millis(500)).unwrap();
        recorder.finish()
    })
    .await
    .unwrap();
    let entries = har["log"]["entries"].as_array().unwrap();
    let urls: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{} {}",
                e["request"]["method"].as_str().unwrap(),
                e["request"]["url"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        urls,
        [
            format!("GET {base}/board"),
            format!("GET {base}/api/items?q=x"),
            format!("POST {base}/api/items"),
        ],
        "{urls:?}"
    );
    let list = &entries[1];
    assert!(
        list["request"]["headers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["name"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case("x-api-key")
                && h["value"] == "recorded-key")
    );
    assert_eq!(list["response"]["status"], 200);
    assert_eq!(list["response"]["content"]["mimeType"], "application/json");
    assert!(
        list["response"]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("\"total\":1")
    );
    let create = &entries[2];
    assert_eq!(create["response"]["status"], 201);
    assert_eq!(create["request"]["postData"]["text"], "{\"title\":\"hi\"}");
    assert_eq!(
        create["request"]["postData"]["mimeType"],
        "application/json"
    );

    // The recording drafts the same way a DevTools export does.
    let draft = itda_prep::from_har(
        &serde_json::to_vec(&har).unwrap(),
        &itda_prep::Options {
            name: Some("board".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(draft.yaml.contains("name: list-items"), "{}", draft.yaml);
    assert!(draft.yaml.contains("name: create-item"), "{}", draft.yaml);
    assert!(!draft.yaml.contains("recorded-key"));
    let _ = std::fs::remove_dir_all(dir);
}
