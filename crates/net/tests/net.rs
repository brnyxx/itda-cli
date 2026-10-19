//! Synthetic local servers only: these tests never contact a real website.
#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect},
    routing::get,
};
use itda_net::{Gate, Network, Request, USER_AGENT, gate};
use itda_spec::ResponseLike;
use itda_store::Store;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn host(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    address
}

fn network(allow_local: bool) -> Network {
    let store = Store::in_memory().unwrap();
    Network::new(
        Arc::new(Gate::with_limits(store, 0, 1000).unwrap()),
        allow_local,
    )
}

#[tokio::test]
async fn reads_json_and_identifies_itself() {
    let base = host(Router::new().route(
        "/api/me",
        get(|headers: HeaderMap| async move {
            let ua = headers[header::USER_AGENT].to_str().unwrap().to_owned();
            assert!(headers.get("sec-ch-ua").is_none());
            axum::Json(serde_json::json!({"ua": ua}))
        }),
    ))
    .await;
    let response = network(true)
        .fetch(Request::read(format!("{base}/api/me")))
        .await
        .unwrap();
    assert!(response.successful());
    assert_eq!(ResponseLike::status(&response), 200);
    assert_eq!(response.json().unwrap()["ua"], USER_AGENT);
}

#[tokio::test]
async fn private_addresses_need_explicit_permission() {
    let base = host(Router::new().route("/", get(|| async { "ok" }))).await;
    let error = network(false).fetch(Request::read(base)).await.unwrap_err();
    assert_eq!(error.public_code, "unsafe_target");
}

#[tokio::test]
async fn redirects_stay_on_origin_and_only_for_reads() {
    let other = host(Router::new().route("/landing", get(|| async { "elsewhere" }))).await;
    let other_url = format!("{other}/landing");
    let base = host(
        Router::new()
            .route("/old", get(|| async { Redirect::temporary("/new") }))
            .route("/new", get(|| async { "moved" }))
            .route(
                "/logout-redirect",
                get(|| async { Redirect::temporary("/logout") }),
            )
            .route(
                "/away",
                get(move || {
                    let target = other_url.clone();
                    async move { Redirect::temporary(&target) }
                }),
            ),
    )
    .await;
    let net = network(true);

    let followed = net
        .fetch(Request::read(format!("{base}/old")))
        .await
        .unwrap();
    assert_eq!(followed.status, 200);
    assert_eq!(followed.bytes, b"moved");

    let cross = net
        .fetch(Request::read(format!("{base}/away")))
        .await
        .unwrap();
    assert_eq!(
        cross.status, 307,
        "cross-origin redirect must not be followed"
    );

    let unsafe_target = net
        .fetch(Request::read(format!("{base}/logout-redirect")))
        .await
        .unwrap();
    assert_eq!(
        unsafe_target.status, 307,
        "redirect to a change must not be followed"
    );

    let mut write = Request::read(format!("{base}/old"));
    write.read_only = false;
    assert_eq!(net.fetch(write).await.unwrap().status, 307);
}

#[tokio::test]
async fn oversized_bodies_are_refused() {
    let base = host(Router::new().route("/big", get(|| async { "x".repeat(10_000) }))).await;
    let mut request = Request::read(format!("{base}/big"));
    request.limit = 1_000;
    let error = network(true).fetch(request).await.unwrap_err();
    assert_eq!(error.public_code, "too_large");
}

#[tokio::test]
async fn a_challenge_pauses_the_site_until_the_user_resumes() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let base = host(Router::new().route(
        "/page",
        get(move || {
            let n = counted.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    (
                        StatusCode::FORBIDDEN,
                        [("cf-mitigated", "challenge")],
                        "Just a moment...",
                    )
                        .into_response()
                } else {
                    "welcome".into_response()
                }
            }
        }),
    ))
    .await;
    let net = network(true);
    let first = net
        .fetch(Request::read(format!("{base}/page")))
        .await
        .unwrap();
    assert_eq!(first.defense, "cloudflare");
    assert!(!first.successful());

    // The same site is not contacted again, for any account, until resumed.
    let paused = net
        .fetch(Request::read(format!("{base}/page")))
        .await
        .unwrap_err();
    assert_eq!(paused.public_code, gate::BLOCKED);
    let mut signed_in = Request::read(format!("{base}/page"));
    signed_in.account_id = "acct".into();
    assert_eq!(
        net.fetch(signed_in).await.unwrap_err().public_code,
        gate::BLOCKED
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let origin = url::Url::parse(&base)
        .unwrap()
        .origin()
        .ascii_serialization();
    net.resume(&origin, "").await.unwrap();
    let again = net
        .fetch(Request::read(format!("{base}/page")))
        .await
        .unwrap();
    assert!(again.successful());
}

#[tokio::test]
async fn an_expired_session_pauses_only_that_account() {
    let base =
        host(Router::new().route("/api/me", get(|| async { StatusCode::UNAUTHORIZED }))).await;
    let net = network(true);
    let mut signed_in = Request::read(format!("{base}/api/me"));
    signed_in.account_id = "acct".into();
    assert_eq!(net.fetch(signed_in).await.unwrap().status, 401);

    let mut retry = Request::read(format!("{base}/api/me"));
    retry.account_id = "acct".into();
    assert_eq!(
        net.fetch(retry).await.unwrap_err().public_code,
        gate::AUTH_REQUIRED
    );
    // Anonymous reads of the same site are unaffected.
    assert_eq!(
        net.fetch(Request::read(format!("{base}/api/me")))
            .await
            .unwrap()
            .status,
        401
    );
}

#[tokio::test]
async fn rate_limits_set_a_cooldown() {
    let base = host(Router::new().route(
        "/limited",
        get(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "60")],
                "slow down",
            )
        }),
    ))
    .await;
    let net = network(true);
    let first = net
        .fetch(Request::read(format!("{base}/limited")))
        .await
        .unwrap();
    assert_eq!(first.status, 429);
    assert_eq!(first.defense, "rate-limit");
    let second = net
        .fetch(Request::read(format!("{base}/limited")))
        .await
        .unwrap_err();
    assert_eq!(second.public_code, "rate_limited");
}

#[tokio::test]
async fn the_budget_caps_requests_per_scope() {
    let gate = Gate::with_limits(Store::in_memory().unwrap(), 0, 2).unwrap();
    for _ in 0..2 {
        let token = gate.acquire("https://example.test|").await.unwrap();
        gate.release("https://example.test|", &token, itda_net::Outcome::Done)
            .await
            .unwrap();
    }
    assert_eq!(
        gate.acquire("https://example.test|")
            .await
            .unwrap_err()
            .public_code,
        "rate_limited"
    );
    // Other scopes are independent.
    gate.acquire("https://other.test|").await.unwrap();
}
