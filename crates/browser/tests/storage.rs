//! Reads a real browser's page storage. Runs only when `ITDA_TEST_BROWSER`
//! names a Chrome or Chromium binary, which is started headless through a
//! small wrapper script.
#![allow(clippy::unwrap_used)]
#![cfg(unix)]
use axum::{Router, response::Html, routing::get};
use std::{os::unix::fs::PermissionsExt, time::Duration};

/// A JSON Web Token `{"sub":"u-1","exp":4102444800}`; the signature is not
/// checked, so any text stands in for it.
const JWT: &str =
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1LTEiLCJleHAiOjQxMDI0NDQ4MDB9.sig";

fn wrapper(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let binary = std::env::var_os("ITDA_TEST_BROWSER").filter(|b| !b.is_empty())?;
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
    Some(wrapper)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_kept_in_local_storage_and_sent_as_a_bearer_header_is_the_session() {
    let Some(binary) = std::env::var_os("ITDA_TEST_BROWSER").filter(|b| !b.is_empty()) else {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    };
    let _ = &binary;
    // The sign-in page keeps a token in localStorage and its script then
    // sends it as `Authorization: Bearer` to the API, as a single-page app
    // that has no session cookie does. A second value is kept in
    // sessionStorage but never sent: it must not be taken for the session.
    let router = Router::new()
        .route(
            "/app",
            get(|| async {
                Html(format!(
                    "<title>App</title><script>
                     localStorage.setItem('access_token', '{JWT}');
                     localStorage.setItem('theme', 'dark');
                     sessionStorage.setItem('scratch', 'never-sent-never-sent');
                     fetch('/api/items', {{headers: {{'Authorization': 'Bearer {JWT}'}}}});
                     </script>"
                ))
            }),
        )
        .route(
            "/api/items",
            get(|| async { axum::Json(serde_json::json!({"items": []})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let dir = std::env::temp_dir().join(format!("itda-storage-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let browser = wrapper(&dir).unwrap();

    let page = url::Url::parse(&format!("{base}/app")).unwrap();
    let origin = page.origin();
    let opened = page.clone();
    let (storage, har) = tokio::task::spawn_blocking(move || {
        let mut recorder = itda_browser::Recorder::open(&browser, &opened).unwrap();
        for _ in 0..60 {
            recorder.poll(Duration::from_millis(250)).unwrap();
            if !recorder.is_empty() {
                break;
            }
        }
        recorder.poll(Duration::from_millis(500)).unwrap();
        let storage = recorder.storage(&opened).unwrap();
        (storage, recorder.finish())
    })
    .await
    .unwrap();

    // Both localStorage items are read, from the signed-in origin only.
    assert!(
        storage
            .iter()
            .any(|item| item.key == "access_token" && item.value == JWT && item.local),
        "the token was read from localStorage: {storage:?}"
    );
    assert!(storage.iter().any(|item| item.key == "theme"));
    // The bearer the page sent picks the token out as the session: the key
    // it was under, the header, and the Bearer prefix.
    let session = itda_browser::bearer_session(&har, &storage, &origin).unwrap();
    assert_eq!(session.key, "access_token");
    assert_eq!(session.header, "Authorization");
    assert_eq!(session.prefix, "Bearer ");
    assert_eq!(session.value, JWT);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_value_no_request_sent_is_not_taken_for_a_session() {
    let Some(_) = std::env::var_os("ITDA_TEST_BROWSER").filter(|b| !b.is_empty()) else {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    };
    // The page keeps a token in storage but its calls carry no header: there
    // is no evidence it is the session, so nothing is recognised.
    let router = Router::new()
        .route(
            "/app",
            get(|| async {
                Html(format!(
                    "<title>App</title><script>
                     localStorage.setItem('leftover', '{JWT}');
                     fetch('/api/items');
                     </script>"
                ))
            }),
        )
        .route(
            "/api/items",
            get(|| async { axum::Json(serde_json::json!({"items": []})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let dir = std::env::temp_dir().join(format!("itda-storage-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let browser = wrapper(&dir).unwrap();

    let page = url::Url::parse(&format!("{base}/app")).unwrap();
    let origin = page.origin();
    let opened = page.clone();
    let (storage, har) = tokio::task::spawn_blocking(move || {
        let mut recorder = itda_browser::Recorder::open(&browser, &opened).unwrap();
        for _ in 0..60 {
            recorder.poll(Duration::from_millis(250)).unwrap();
            if !recorder.is_empty() {
                break;
            }
        }
        recorder.poll(Duration::from_millis(500)).unwrap();
        let storage = recorder.storage(&opened).unwrap();
        (storage, recorder.finish())
    })
    .await
    .unwrap();

    assert!(storage.iter().any(|item| item.key == "leftover"));
    assert_eq!(
        itda_browser::bearer_session(&har, &storage, &origin),
        None,
        "a stored value no request sent is not the session"
    );
    let _ = std::fs::remove_dir_all(dir);
}
