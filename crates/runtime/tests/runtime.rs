//! Synthetic local servers only: these tests never contact a real website.
#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    extract::Query,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use itda_net::{Gate, Network};
use itda_runtime::{Call, Runtime};
use itda_spec::Spec;
use itda_store::Store;
use itda_vault::{MemoryStore, Secret, Vault};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const KEY: &str = "fixture-api-key-123";

struct Site {
    base: String,
    hits: Arc<AtomicUsize>,
}

async fn site() -> Site {
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = |hits: Arc<AtomicUsize>| {
        move || {
            hits.fetch_add(1, Ordering::SeqCst);
        }
    };
    let (a, b, c, d) = (
        counted(hits.clone()),
        counted(hits.clone()),
        counted(hits.clone()),
        counted(hits.clone()),
    );
    let router = Router::new()
        .route(
            "/items",
            get(move |Query(q): Query<HashMap<String, String>>| async move {
                a();
                axum::Json(json!({"items": [{"title": format!("first {}", q.get("q").cloned().unwrap_or_default())}], "total": 1}))
            })
            .post(move || async move {
                b();
                StatusCode::CREATED
            }),
        )
        .route(
            "/me",
            get(move |headers: HeaderMap| async move {
                c();
                match headers.get("x-key").and_then(|v| v.to_str().ok()) {
                    Some(KEY) => Ok(axum::Json(
                        json!({"name": "Mina", "note": format!("signed in with {KEY}")}),
                    )),
                    _ => Err(StatusCode::UNAUTHORIZED),
                }
            }),
        )
        .route(
            "/welcome",
            get(move || async move {
                d();
                axum::response::Html("<form>Sign in</form>")
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Site { base, hits }
}

fn spec(base: &str) -> Spec {
    let value = json!({
        "spec_version": 1,
        "site": {"name": "fixture", "base_url": base},
        "auth": [{"name": "key", "kind": "header", "header": "X-Key", "value_from": "store:fixture/api-key"}],
        "operations": [
            {"name": "list-items", "method": "GET", "path": "/items", "effect": "read", "evidence": "doc",
             "query": {"q": {"type": "string"}},
             "response": {"format": "json", "required_pointers": ["/items"]}},
            {"name": "create-item", "method": "POST", "path": "/items", "effect": "write", "evidence": "doc",
             "body": {"json": {"title": {"type": "string"}}}},
            {"name": "me", "method": "GET", "path": "/me", "auth": "key", "effect": "read", "evidence": "doc",
             "response": {"format": "json", "required_pointers": ["/name"]}},
            {"name": "profile", "method": "GET", "path": "/welcome", "effect": "read", "evidence": "doc",
             "response": {"format": "json", "required_pointers": ["/name"]}}
        ]
    });
    itda_spec::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
}

async fn runtime() -> (Runtime, Arc<Vault>) {
    let store = Store::in_memory().unwrap();
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    (
        Runtime::new(Arc::new(Network::new(gate, true)), vault.clone()),
        vault,
    )
}

fn call(action: &str, args: &[(&str, &str)]) -> Call {
    Call {
        action: action.into(),
        args: args
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect::<BTreeMap<_, _>>(),
        account: None,
    }
}

#[tokio::test]
async fn reads_return_an_envelope() {
    let site = site().await;
    let (runtime, _) = runtime().await;
    let envelope = runtime
        .call(&spec(&site.base), call("list-items", &[("q", "books")]))
        .await
        .unwrap();
    assert_eq!(envelope.site, "fixture");
    assert_eq!(envelope.effect, "read");
    assert_eq!(envelope.data["items"][0]["title"], "first books");
    assert_eq!(envelope.meta.http_status, 200);
    assert!(envelope.account.is_none());
}

#[tokio::test]
async fn changes_and_unknown_inputs_never_reach_the_site() {
    let site = site().await;
    let (runtime, _) = runtime().await;
    let spec = spec(&site.base);
    let error = runtime
        .call(&spec, call("create-item", &[("title", "x")]))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "writes_disabled");
    let error = runtime
        .call(&spec, call("list-items", &[("page", "2")]))
        .await
        .unwrap_err();
    assert!(error.msg.contains("Unknown input: page"));
    assert_eq!(site.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn credentials_come_from_the_vault_and_stay_out_of_results() {
    let site = site().await;
    let (runtime, vault) = runtime().await;
    let spec = spec(&site.base);

    // No account yet: ask to reconnect without sending anything.
    let error = runtime.call(&spec, call("me", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "auth_required");
    assert_eq!(site.hits.load(Ordering::SeqCst), 0);

    vault.add_account("fixture", "default", "Me").await.unwrap();
    vault
        .set_secret("fixture", "default", "api-key", Secret::new(KEY))
        .await
        .unwrap();
    let envelope = runtime.call(&spec, call("me", &[])).await.unwrap();
    assert_eq!(envelope.data["name"], "Mina");
    assert_eq!(envelope.account.as_deref(), Some("default"));
    let text = serde_json::to_string(&envelope).unwrap();
    assert!(!text.contains(KEY), "{text}");
}

#[tokio::test]
async fn a_rejected_key_pauses_the_account() {
    let site = site().await;
    let (runtime, vault) = runtime().await;
    let spec = spec(&site.base);
    vault.add_account("fixture", "default", "").await.unwrap();
    vault
        .set_secret(
            "fixture",
            "default",
            "api-key",
            Secret::new("wrong-key-value"),
        )
        .await
        .unwrap();
    let error = runtime.call(&spec, call("me", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "auth_required");
    // The next call is refused by the gate without contacting the site.
    let before = site.hits.load(Ordering::SeqCst);
    let error = runtime.call(&spec, call("me", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "auth_required");
    assert_eq!(site.hits.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn a_page_that_is_not_the_declared_response_is_reported() {
    let site = site().await;
    let (runtime, _) = runtime().await;
    let error = runtime
        .call(&spec(&site.base), call("profile", &[]))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "response_mismatch");
}
