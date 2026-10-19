//! Opens a real browser against a local page. Runs only when
//! `ITDA_TEST_BROWSER` names a Chrome or Chromium binary, which is started
//! headless through a small wrapper script.
#![allow(clippy::unwrap_used)]
#![cfg(unix)]
use axum::{
    Router,
    http::header::SET_COOKIE,
    response::{AppendHeaders, IntoResponse},
    routing::get,
};
use std::{os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test(flavor = "multi_thread")]
async fn keeps_only_the_cookies_for_the_signed_in_site() {
    let Some(binary) = std::env::var_os("ITDA_TEST_BROWSER").filter(|b| !b.is_empty()) else {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    };
    let router = Router::new().route(
        "/signed-in",
        get(|| async {
            (
                AppendHeaders([
                    (SET_COOKIE, "sid=abc123; Path=/; HttpOnly"),
                    (SET_COOKIE, "theme=dark; Path=/"),
                    (SET_COOKIE, "old=gone; Path=/; Max-Age=0"),
                ]),
                "signed in",
            )
                .into_response()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let dir = std::env::temp_dir().join(format!("itda-browser-test-{}", std::process::id()));
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

    let page = url::Url::parse(&format!("{base}/signed-in")).unwrap();
    let cookies = tokio::task::spawn_blocking(move || {
        let mut browser = itda_browser::SignIn::open(&wrapper, &page).unwrap();
        for _ in 0..50 {
            let cookies = browser.cookies(&page).unwrap();
            if cookies.iter().any(|c| c.name == "sid") {
                return cookies;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the sign-in cookie never appeared");
    })
    .await
    .unwrap();
    let mut names: Vec<&str> = cookies.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["sid", "theme"]);
    assert!(itda_browser::header(&cookies).contains("sid=abc123"));
    let _ = std::fs::remove_dir_all(dir);
}
