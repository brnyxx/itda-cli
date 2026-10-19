//! Synthetic local servers only: these tests never contact a real website.
#![allow(clippy::unwrap_used)]
use itda_net::Gate;

#[tokio::test]
async fn the_budget_caps_requests_per_scope() {
    let path = std::env::temp_dir()
        .join(format!("itda-gate-{}", itda_types::util::id()))
        .join("gate.db");
    let gate = Gate::with_limits(&path, 0, 2).unwrap();
    for _ in 0..2 {
        let token = gate.acquire("https://example.test|").await.unwrap();
        gate.release("https://example.test|", &token, itda_net::Outcome::Done)
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
