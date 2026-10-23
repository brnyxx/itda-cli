use itda_browser::broker::{BrokerConfig, BrowserClient, BrowserCompletion};
use std::path::PathBuf;

#[test]
fn construction_and_reset_do_not_start_a_worker() {
    let state_dir = std::env::temp_dir().join(format!("itda-broker-{}", uuid::Uuid::new_v4()));
    let client = BrowserClient::broker(BrokerConfig {
        state_dir: state_dir.clone(),
        executable: PathBuf::from("/no/such/browser-worker"),
        browser: None,
        allow_local: false,
    });
    let clone = client.clone();
    assert!(!state_dir.exists());
    clone
        .reset("https://example.com", "fixture")
        .expect("reset");
    clone.shutdown().expect("shutdown");
    assert!(!state_dir.exists());
    assert_ne!(BrowserCompletion::NotStarted, BrowserCompletion::Unknown);
}
