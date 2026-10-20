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
const SID: &str = "fixture-session-456";

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
            "/account",
            get(|headers: HeaderMap| async move {
                let cookie = headers
                    .get("cookie")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                if cookie.contains(&format!("sid={SID}")) {
                    Ok(axum::Json(json!({"name": "Mina", "echo": cookie})))
                } else {
                    Err(StatusCode::UNAUTHORIZED)
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
        "auth": [{"name": "key", "kind": "header", "header": "X-Key", "value_from": "store:fixture/api-key"},
                 {"name": "session", "kind": "browser", "value_from": "cookie:sid"}],
        "operations": [
            {"name": "list-items", "method": "GET", "path": "/items", "effect": "read", "evidence": "doc",
             "query": {"q": {"type": "string"}},
             "response": {"format": "json", "required_pointers": ["/items"]}},
            {"name": "create-item", "method": "POST", "path": "/items", "effect": "write", "evidence": "doc",
             "body": {"json": {"title": {"type": "string"}}}},
            {"name": "me", "method": "GET", "path": "/me", "auth": "key", "effect": "read", "evidence": "doc",
             "response": {"format": "json", "required_pointers": ["/name"]}},
            {"name": "account", "method": "GET", "path": "/account", "auth": "session", "effect": "read", "evidence": "doc",
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
        approval: None,
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
async fn a_browser_session_goes_to_its_site_and_stays_out_of_results() {
    let site = site().await;
    let (runtime, vault) = runtime().await;
    let spec = spec(&site.base);

    let error = runtime.call(&spec, call("account", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "auth_required");

    // A jar without the sign-in cookie is a signed-out session: nothing is sent.
    vault.add_account("fixture", "default", "").await.unwrap();
    vault
        .set_secret("fixture", "default", "session", Secret::new("theme=dark"))
        .await
        .unwrap();
    let error = runtime.call(&spec, call("account", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "auth_required");
    assert_eq!(site.hits.load(Ordering::SeqCst), 0);

    vault
        .set_secret(
            "fixture",
            "default",
            "session",
            Secret::new(format!("theme=dark; sid={SID}")),
        )
        .await
        .unwrap();
    let envelope = runtime.call(&spec, call("account", &[])).await.unwrap();
    assert_eq!(envelope.data["name"], "Mina");
    let text = serde_json::to_string(&envelope).unwrap();
    assert!(!text.contains(SID), "{text}");
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

#[tokio::test]
async fn a_change_runs_once_with_the_users_approval_for_exactly_those_inputs() {
    let site = site().await;
    let store = Store::in_memory().unwrap();
    let approvals = Arc::new(itda_policy::Approvals::open(store.clone()).unwrap());
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    let runtime =
        Runtime::new(Arc::new(Network::new(gate, true)), vault).with_approvals(approvals.clone());
    let spec = spec(&site.base);
    let with = |title: &str, approval: Option<&str>| {
        let mut call = call("create-item", &[("title", title)]);
        call.approval = approval.map(str::to_owned);
        call
    };

    // The first call only records the request.
    let error = runtime.call(&spec, with("hello", None)).await.unwrap_err();
    assert_eq!(error.public_code, "approval_required");
    let id = error.extra["approval"].as_str().unwrap().to_owned();
    let review = approvals.review(&id).await.unwrap();
    assert_eq!(review.inputs[0].value, format!("POST {}/items", site.base));
    assert_eq!(review.inputs[1].label, "title");

    // The agent cannot use it before the user grants it.
    let error = runtime
        .call(&spec, with("hello", Some(&id)))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "approval_expired");
    let error = runtime.call(&spec, with("hello", None)).await.unwrap_err();
    let id = error.extra["approval"].as_str().unwrap().to_owned();
    approvals.claim(&id).await.unwrap();

    // Other inputs do not match what the user saw.
    let error = runtime
        .call(&spec, with("other", Some(&id)))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "approval_expired");
    assert_eq!(site.hits.load(Ordering::SeqCst), 0);

    // A mismatch does not use the approval up; the exact call works once.
    let envelope = runtime.call(&spec, with("hello", Some(&id))).await.unwrap();
    assert_eq!(envelope.effect, "write");
    assert_eq!(envelope.meta.http_status, 201);
    let error = runtime
        .call(&spec, with("hello", Some(&id)))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "approval_expired");
    assert_eq!(site.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_audit_log_records_calls_and_changes_before_they_are_sent() {
    use itda_runtime::audit::AuditLog;
    let site = site().await;
    let store = Store::in_memory().unwrap();
    let approvals = Arc::new(itda_policy::Approvals::open(store.clone()).unwrap());
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    let dir = std::env::temp_dir().join(format!("itda-runtime-audit-{}", itda_types::util::id()));
    let log = Arc::new(AuditLog::new(dir.join("audit.jsonl")));
    let runtime = Runtime::new(Arc::new(Network::new(gate, true)), vault)
        .with_approvals(approvals.clone())
        .with_audit(log.clone());
    let spec = spec(&site.base);

    runtime.call(&spec, call("list-items", &[])).await.unwrap();
    let error = runtime
        .call(&spec, call("create-item", &[("title", "private words")]))
        .await
        .unwrap_err();
    let id = error.extra["approval"].as_str().unwrap().to_owned();
    approvals.claim(&id).await.unwrap();
    let mut approved = call("create-item", &[("title", "private words")]);
    approved.approval = Some(id.clone());
    runtime.call(&spec, approved).await.unwrap();

    let records = log.records().unwrap();
    let summary: Vec<(&str, &str)> = records
        .iter()
        .map(|r| (r.kind.as_str(), r.outcome.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            ("call", "ok"),
            ("call", "approval_required"),
            ("change_started", "sending"),
            ("call", "ok")
        ]
    );
    assert_eq!(records[1].approval, id);
    assert_eq!(records[3].effect, "write");
    assert_eq!(log.verify().unwrap(), 4);
    assert!(
        !std::fs::read_to_string(log.path())
            .unwrap()
            .contains("private words")
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn preflight_steps_fetch_a_fresh_token_for_the_change() {
    const TOKEN: &str = "csrf-token-7f3a91";
    let posted = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = posted.clone();
    let router = Router::new()
        .route(
            "/form",
            get(|| async {
                axum::response::Html(format!(
                    "<form><input type=hidden name=_csrf value={TOKEN}></form>"
                ))
            }),
        )
        .route(
            "/empty",
            get(|| async { axum::response::Html("<form></form>") }),
        )
        .route(
            "/posts",
            axum::routing::post(move |headers: HeaderMap, body: String| async move {
                let token = headers
                    .get("x-csrf-token")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                assert!(body.contains(&format!("_csrf={TOKEN}")), "{body}");
                seen.lock().unwrap().push(token.clone());
                axum::Json(json!({"ok": true, "echo": token}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let value = json!({
        "spec_version": 1,
        "site": {"name": "board", "base_url": base},
        "operations": [
            {"name": "post", "method": "POST", "path": "/posts", "effect": "write", "evidence": "doc",
             "headers": {"X-CSRF-Token": "{{step.form.csrf}}"},
             "preflight": [{"name": "form", "path": "/form", "extract": {"csrf": "css:input[name=_csrf]@value"}}],
             "body": {"form": {"title": {"type": "string"}, "_csrf": {"type": "string", "default": "{{step.form.csrf}}"}}}},
            {"name": "broken", "method": "POST", "path": "/posts", "effect": "write", "evidence": "doc",
             "headers": {"X-CSRF-Token": "{{step.form.csrf}}"},
             "preflight": [{"name": "form", "path": "/empty", "extract": {"csrf": "css:input[name=_csrf]@value"}}]}
        ]
    });
    let spec = itda_spec::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let store = Store::in_memory().unwrap();
    let approvals = Arc::new(itda_policy::Approvals::open(store.clone()).unwrap());
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    let runtime =
        Runtime::new(Arc::new(Network::new(gate, true)), vault).with_approvals(approvals.clone());
    let approved = |action: &'static str, args: &'static [(&'static str, &'static str)]| {
        let runtime = &runtime;
        let approvals = approvals.clone();
        let spec = &spec;
        async move {
            let error = runtime.call(spec, call(action, args)).await.unwrap_err();
            let id = error.extra["approval"].as_str().unwrap().to_owned();
            approvals.claim(&id).await.unwrap();
            let mut with = call(action, args);
            with.approval = Some(id);
            runtime.call(spec, with).await
        }
    };

    let envelope = approved("post", &[("title", "hi")]).await.unwrap();
    assert_eq!(posted.lock().unwrap().as_slice(), [TOKEN]);
    // The token is the site's, not data for the agent.
    assert!(!envelope.data.to_string().contains(TOKEN));

    // The agent cannot supply what a step fills in.
    let error = runtime
        .call(&spec, call("post", &[("_csrf", "forged")]))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "bad_request");

    let error = approved("broken", &[]).await.unwrap_err();
    assert_eq!(error.public_code, "response_mismatch");
    assert_eq!(posted.lock().unwrap().len(), 1);
}

#[test]
fn preflight_steps_only_read() {
    let value = json!({
        "spec_version": 1,
        "site": {"name": "board", "base_url": "https://board.example.com"},
        "operations": [
            {"name": "post", "method": "POST", "path": "/posts", "effect": "write", "evidence": "doc",
             "preflight": [{"name": "form", "method": "POST", "path": "/form", "extract": {"title": "css:input", "x": "body:/a"}}],
             "body": {"json": {"title": {"type": "string"}}}}
        ]
    });
    let spec = itda_spec::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let error = itda_policy::validate_spec(&spec).unwrap_err();
    let text = format!("{} {}", error.msg, error.remedy);
    assert!(text.contains("preflight"), "{text}");
}
