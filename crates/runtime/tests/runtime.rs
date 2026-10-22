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
use serde_json::{Value, json};
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
        next: None,
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
    assert_eq!(review.action_title, "create-item");
    assert_eq!(review.inputs[0].value, format!("POST {}/items", site.base));
    assert_eq!(review.inputs[1].label, "title");

    // Before the user answers, the agent is told to wait; the approval stays.
    let error = runtime
        .call(&spec, with("hello", Some(&id)))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "approval_pending");
    assert_eq!(error.extra["approval"], id.as_str());
    assert_eq!(error.extra["command"], format!("itda approve {id}"));
    assert!(
        error.remedy.contains("do not request a new one"),
        "{}",
        error.remedy
    );
    assert_eq!(approvals.review(&id).await.unwrap().status, "pending");
    approvals.claim(&id).await.unwrap();

    // Other inputs are not what the user saw: the approval is withdrawn.
    let error = runtime
        .call(&spec, with("other", Some(&id)))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "approval_expired");
    assert_eq!(error.extra["reason"], "inputs_changed");
    assert_eq!(site.hits.load(Ordering::SeqCst), 0);
    let error = runtime
        .call(&spec, with("hello", Some(&id)))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "approval_expired");
    assert!(error.extra.get("reason").is_none());

    // A fresh approval for the exact call works once.
    let error = runtime.call(&spec, with("hello", None)).await.unwrap_err();
    let id = error.extra["approval"].as_str().unwrap().to_owned();
    approvals.claim(&id).await.unwrap();
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

/// A server that takes the request and then drops the connection, so the
/// client never learns what happened to it.
async fn dropping_site() -> Site {
    use tokio::io::AsyncReadExt;
    let hits = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            drop(socket);
        }
    });
    Site { base, hits }
}

#[tokio::test]
async fn a_change_whose_answer_is_lost_has_an_unknown_outcome_and_is_not_resent() {
    use itda_runtime::audit::AuditLog;
    let site = dropping_site().await;
    let store = Store::in_memory().unwrap();
    let approvals = Arc::new(itda_policy::Approvals::open(store.clone()).unwrap());
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    let dir = std::env::temp_dir().join(format!("itda-runtime-lost-{}", itda_types::util::id()));
    let log = Arc::new(AuditLog::new(dir.join("audit.jsonl")));
    let runtime = Runtime::new(Arc::new(Network::new(gate, true)), vault)
        .with_approvals(approvals.clone())
        .with_audit(log.clone());
    let spec = spec(&site.base);

    let error = runtime
        .call(&spec, call("create-item", &[("title", "hello")]))
        .await
        .unwrap_err();
    let id = error.extra["approval"].as_str().unwrap().to_owned();
    approvals.claim(&id).await.unwrap();
    let mut approved = call("create-item", &[("title", "hello")]);
    approved.approval = Some(id);
    let error = runtime.call(&spec, approved).await.unwrap_err();
    assert_eq!(error.public_code, "action_outcome_unknown");
    assert!(error.remedy.contains("never resends"), "{}", error.remedy);
    assert_eq!(site.hits.load(Ordering::SeqCst), 1);

    let records = log.records().unwrap();
    let last = records.last().unwrap();
    assert_eq!(last.kind, "call");
    assert_eq!(last.outcome, "action_outcome_unknown");
    assert_eq!(records[records.len() - 2].outcome, "sending");
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_read_whose_answer_is_lost_keeps_its_network_code() {
    let site = dropping_site().await;
    let (runtime, _) = runtime().await;
    let error = runtime
        .call(&spec(&site.base), call("list-items", &[]))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "request_failed");
}

#[tokio::test]
async fn a_rate_limited_read_says_how_long_the_site_asked_to_wait() {
    let router = Router::new().route(
        "/items",
        get(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "120")],
                "slow down",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (runtime, _) = runtime().await;
    let error = runtime
        .call(&spec(&base), call("list-items", &[]))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "site_rate_limited");
    assert_eq!(error.extra["retry_after"], 120);
    assert!(error.msg.contains("wait 120 s"), "{}", error.msg);
}

/// A site whose lists turn pages four ways: a cursor in the body, a page
/// number with a `Link` header, an offset with no marker at all, and a
/// GraphQL connection with `pageInfo`.
async fn paged_site() -> String {
    const PAGE: usize = 2;
    let records = |from: usize, to: usize| -> Vec<Value> {
        (from..to.min(5))
            .map(|i| json!({"id": i, "title": format!("record {i}")}))
            .collect()
    };
    let router = Router::new()
        .route(
            "/cursor",
            get(move |Query(q): Query<HashMap<String, String>>| async move {
                let from: usize = q
                    .get("cursor")
                    .map(|c| c.strip_prefix("cur-").unwrap_or("0").parse().unwrap())
                    .unwrap_or(0);
                let next = (from + PAGE < 5).then(|| format!("cur-{}", from + PAGE));
                axum::Json(json!({"meta": {"next_cursor": next, "echo": q.get("cursor")}, "items": records(from, from + PAGE)}))
            }),
        )
        .route(
            "/paged",
            get(move |request_headers: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                let page: usize = q.get("page").map(|p| p.parse().unwrap()).unwrap_or(1);
                let from = (page - 1) * PAGE;
                let host = request_headers["host"].to_str().unwrap().to_owned();
                let mut headers = HeaderMap::new();
                if from + PAGE < 5 {
                    headers.insert(
                        "link",
                        format!("<http://{host}/paged?per_page={PAGE}&page={}>; rel=\"next\", <http://{host}/paged?page=1>; rel=\"first\"", page + 1)
                            .parse()
                            .unwrap(),
                    );
                }
                (headers, axum::Json(json!({"items": records(from, from + PAGE)})))
            }),
        )
        .route(
            "/offset",
            get(move |Query(q): Query<HashMap<String, String>>| async move {
                let from: usize = q.get("from").map(|p| p.parse().unwrap()).unwrap_or(0);
                axum::Json(Value::Array(records(from, from + PAGE)))
            }),
        )
        .route(
            "/graphql",
            axum::routing::post(move |body: String| async move {
                let body: Value = serde_json::from_str(&body).unwrap();
                let after = body["variables"]["after"].as_str();
                let from: usize = after.map(|c| c.strip_prefix("gql-").unwrap().parse().unwrap()).unwrap_or(0);
                axum::Json(json!({"data": {"notices": {
                    "nodes": records(from, from + PAGE),
                    "pageInfo": {"endCursor": format!("gql-{}", from + PAGE), "hasNextPage": from + PAGE < 5}
                }}}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn paged_spec(base: &str) -> Spec {
    let value = json!({
        "spec_version": 1,
        "site": {"name": "paged", "base_url": base},
        "operations": [
            {"name": "by-cursor", "method": "GET", "path": "/cursor", "effect": "read", "evidence": "doc",
             "query": {"cursor": {"type": "string"}},
             "pagination": {"style": "cursor", "next": "/meta/next_cursor", "send": {"query": "cursor"}},
             "response": {"format": "json", "extract_pointer": "/items"}},
            {"name": "by-page", "method": "GET", "path": "/paged", "effect": "read", "evidence": "doc",
             "query": {"page": {"type": "int", "default": 1}, "per_page": {"type": "int", "default": 2}},
             "pagination": {"style": "page", "next": "header:link", "send": {"query": "page"}},
             "response": {"format": "json", "required_pointers": ["/items"]}},
            {"name": "by-offset", "method": "GET", "path": "/offset", "effect": "read", "evidence": "doc",
             "query": {"from": {"type": "int", "default": 0}},
             "pagination": {"style": "offset", "send": {"query": "from"}},
             "response": {"format": "json"}},
            {"name": "notices", "method": "POST", "path": "/graphql", "effect": "read", "evidence": "doc",
             "body": {"json": {"query": {"type": "string", "default": "query Notices($after: String) { notices(after: $after) { nodes { id title } pageInfo { endCursor hasNextPage } } }"},
                               "after": {"type": "string"}},
                      "json_template": {"query": "{{query}}", "variables": {"after": "{{after}}"}}},
             "pagination": {"style": "cursor", "next": "/data/notices/pageInfo/endCursor", "more": "/data/notices/pageInfo/hasNextPage", "send": {"body": "after"}},
             "response": {"format": "json", "extract_pointer": "/data/notices/nodes"}}
        ]
    });
    itda_spec::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
}

async fn paged_runtime() -> Runtime {
    let store = Store::in_memory().unwrap();
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let cursors = Arc::new(itda_policy::Cursors::open(store.clone()).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    Runtime::new(Arc::new(Network::new(gate, true)), vault).with_pagination(cursors)
}

fn ids(data: &Value) -> Vec<i64> {
    data.as_array()
        .or_else(|| data["items"].as_array())
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_read_that_turns_pages_hands_out_a_handle_and_continues_from_it() {
    let base = paged_site().await;
    let runtime = paged_runtime().await;
    let spec = paged_spec(&base);
    itda_policy::validate_spec(&spec).unwrap();
    for action in ["by-cursor", "by-page", "by-offset", "notices"] {
        let mut seen = Vec::new();
        let mut next = None;
        for _ in 0..4 {
            let mut call = call(action, &[]);
            call.next = next.clone();
            let envelope = runtime.call(&spec, call).await.unwrap();
            seen.extend(ids(&envelope.data));
            assert!(
                !serde_json::to_string(&envelope).unwrap().contains("cur-"),
                "{action}: the raw cursor stays out of the answer"
            );
            next = envelope.meta.next;
            if next.is_none() {
                break;
            }
            assert!(next.as_deref().unwrap().starts_with("ic_"), "{action}");
        }
        assert_eq!(seen, [0, 1, 2, 3, 4], "{action}: three pages, no repeats");
    }
}

#[tokio::test]
async fn a_handle_only_continues_the_call_it_was_issued_for() {
    let base = paged_site().await;
    let runtime = paged_runtime().await;
    let spec = paged_spec(&base);
    let first = runtime.call(&spec, call("by-cursor", &[])).await.unwrap();
    let handle = first.meta.next.clone().unwrap();

    // Another action, or other inputs, is another list.
    let mut other = call("by-page", &[]);
    other.next = Some(handle.clone());
    let error = runtime.call(&spec, other).await.unwrap_err();
    assert_eq!(error.public_code, "pagination_invalid");
    let mut other = call("by-page", &[("per_page", "3")]);
    other.next = Some(handle.clone());
    assert_eq!(
        runtime.call(&spec, other).await.unwrap_err().public_code,
        "pagination_invalid"
    );

    // The agent cannot set a cursor itself, nor make one up as a handle.
    let error = runtime
        .call(&spec, call("by-cursor", &[("cursor", "cur-2")]))
        .await
        .unwrap_err();
    assert!(error.msg.contains("previous answer"), "{}", error.msg);
    let mut made_up = call("by-cursor", &[]);
    made_up.next = Some("cur-2".into());
    let error = runtime.call(&spec, made_up).await.unwrap_err();
    assert_eq!(error.public_code, "pagination_invalid");
    assert!(error.msg.contains("not a handle"), "{}", error.msg);

    // A definition that changed is a different list too.
    let mut changed = spec.clone();
    changed.operations[0].desc = "List records".into();
    let mut again = call("by-cursor", &[]);
    again.next = Some(handle.clone());
    assert_eq!(
        runtime.call(&changed, again).await.unwrap_err().public_code,
        "pagination_invalid"
    );

    // The same handle still works for the call it was issued for.
    let mut same = call("by-cursor", &[]);
    same.next = Some(handle);
    assert_eq!(ids(&runtime.call(&spec, same).await.unwrap().data), [2, 3]);

    // A read that does not turn pages takes no handle, and page and next
    // together are refused.
    let mut plain = call("by-page", &[("page", "2")]);
    plain.next = Some(first.meta.next.unwrap());
    assert_eq!(
        runtime.call(&spec, plain).await.unwrap_err().code,
        itda_types::Code::Usage
    );
}

#[tokio::test]
async fn without_a_handle_store_a_read_still_answers_and_hands_out_no_handle() {
    let base = paged_site().await;
    let (runtime, _) = runtime().await;
    let envelope = runtime
        .call(&paged_spec(&base), call("by-cursor", &[]))
        .await
        .unwrap();
    assert_eq!(ids(&envelope.data), [0, 1]);
    assert!(envelope.meta.next.is_none());
}

#[tokio::test]
async fn a_preflight_step_reads_a_value_from_the_pages_embedded_data() {
    let router = Router::new()
        .route(
            "/",
            get(|| async {
                axum::response::Html(
                    r#"<html><script id="__NEXT_DATA__" type="application/json">{"buildId":"b1ld-7f3a9","props":{"pageProps":{"items":[1]}}}</script></html>"#,
                )
            }),
        )
        .route(
            "/_next/data/b1ld-7f3a9/items.json",
            get(|| async { axum::Json(json!({"pageProps": {"items": [{"id": 1}]}})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let value = json!({
        "spec_version": 1,
        "site": {"name": "next-site", "base_url": base},
        "operations": [
            {"name": "items", "method": "GET", "path": "/_next/data/{{step.page.build_id}}/items.json", "effect": "read", "evidence": "doc",
             "preflight": [{"name": "page", "path": "/", "extract": {"build_id": "ssr:__NEXT_DATA__#/buildId"}}],
             "response": {"format": "json", "required_pointers": ["/pageProps/items"]}},
            {"name": "broken", "method": "GET", "path": "/_next/data/{{step.page.build_id}}/items.json", "effect": "read", "evidence": "doc",
             "preflight": [{"name": "page", "path": "/", "extract": {"build_id": "ssr:__NEXT_DATA__#/props"}}],
             "response": {"format": "json"}}
        ]
    });
    let spec = itda_spec::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();
    let (runtime, _) = runtime().await;
    let envelope = runtime.call(&spec, call("items", &[])).await.unwrap();
    assert_eq!(envelope.data["pageProps"]["items"][0]["id"], 1);
    // An object is not a value.
    let error = runtime.call(&spec, call("broken", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "response_mismatch");
}

/// A site that refuses in the ways real APIs do: a permission problem, an
/// expired token behind a 403, and an address that is gone.
async fn refusing_site() -> String {
    let router = Router::new()
        .route(
            "/denied",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    axum::Json(json!({"error": {"code": "insufficient_scope", "message": "private details"}})),
                )
            }),
        )
        .route(
            "/expired",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    axum::Json(json!({"code": "token-expired"})),
                )
            }),
        )
        .route("/gone", get(|| async { StatusCode::GONE }))
        .route(
            "/graphql",
            axum::routing::post(|body: String| async move {
                if body.contains("mutation") {
                    axum::Json(json!({"data": null, "errors": [{"message": "not allowed"}]}))
                } else {
                    axum::Json(json!({"data": {"items": []}, "errors": [{"message": "bad field"}]}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn refusing_spec(base: &str) -> Spec {
    let value = json!({
        "spec_version": 1,
        "site": {"name": "fixture", "base_url": base},
        "auth": [{"name": "session", "kind": "browser", "value_from": "cookie:sid"}],
        "operations": [
            {"name": "denied", "method": "GET", "path": "/denied", "effect": "read", "evidence": "doc"},
            {"name": "denied-signed-in", "method": "GET", "path": "/denied", "auth": "session", "effect": "read", "evidence": "doc"},
            {"name": "expired", "method": "GET", "path": "/expired", "auth": "session", "effect": "read", "evidence": "doc"},
            {"name": "gone", "method": "GET", "path": "/gone", "effect": "read", "evidence": "doc"},
            {"name": "query-items", "method": "POST", "path": "/graphql", "effect": "read", "evidence": "doc",
             "body": {"json": {"query": {"type": "string", "default": "{ items { id } }"}}},
             "response": {"format": "json", "required_pointers": ["/data"]}},
            {"name": "create-via-graphql", "method": "POST", "path": "/graphql", "effect": "write", "evidence": "doc",
             "body": {"json": {"query": {"type": "string", "default": "mutation { createItem { id } }"}}}}
        ]
    });
    itda_spec::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
}

#[tokio::test]
async fn a_refusal_says_what_the_site_answered_without_its_words() {
    let base = refusing_site().await;
    let spec = refusing_spec(&base);
    let (runtime, vault) = runtime().await;
    vault.add_account("fixture", "default", "").await.unwrap();
    vault
        .set_secret(
            "fixture",
            "default",
            "session",
            Secret::new(format!("sid={SID}")),
        )
        .await
        .unwrap();

    // A 403 is a permission problem, signed in or not, never a sign-in one.
    for action in ["denied", "denied-signed-in"] {
        let error = runtime.call(&spec, call(action, &[])).await.unwrap_err();
        assert_eq!(error.public_code, "api_denied", "{action}");
        assert_eq!(error.extra["upstream_status"], 403);
        assert_eq!(error.extra["provider_reason"], "insufficient_scope");
        assert_eq!(
            error.msg,
            "the site refused this request (insufficient_scope)"
        );
        assert!(!format!("{error:?}").contains("private details"));
        assert!(
            error.remedy.contains("signing in again will not help"),
            "{}",
            error.remedy
        );
    }
    // Unless the site says the session itself is the problem.
    let error = runtime.call(&spec, call("expired", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "auth_required");
    assert_eq!(error.extra["upstream_status"], 403);
    // A gone address keeps its status for `itda check` to read.
    let error = runtime.call(&spec, call("gone", &[])).await.unwrap_err();
    assert_eq!(error.public_code, "request_failed");
    assert_eq!(error.extra["upstream_status"], 410);
}

#[tokio::test]
async fn graphql_errors_behind_a_2xx_are_never_a_success() {
    let base = refusing_site().await;
    let spec = refusing_spec(&base);
    let store = Store::in_memory().unwrap();
    let approvals = Arc::new(itda_policy::Approvals::open(store.clone()).unwrap());
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    let runtime =
        Runtime::new(Arc::new(Network::new(gate, true)), vault).with_approvals(approvals.clone());

    let error = runtime
        .call(&spec, call("query-items", &[]))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "query_failed");
    assert_eq!(error.extra["upstream_status"], 200);

    let error = runtime
        .call(&spec, call("create-via-graphql", &[]))
        .await
        .unwrap_err();
    let id = error.extra["approval"].as_str().unwrap().to_owned();
    approvals.claim(&id).await.unwrap();
    let mut approved = call("create-via-graphql", &[]);
    approved.approval = Some(id);
    let error = runtime.call(&spec, approved).await.unwrap_err();
    assert_eq!(error.public_code, "action_outcome_unknown");
    assert!(
        error.msg.contains("answered the change with errors"),
        "{}",
        error.msg
    );
}

#[tokio::test]
async fn a_change_that_never_left_the_machine_is_reported_as_not_sent() {
    use itda_runtime::audit::AuditLog;
    // A port nobody listens on: the connection is refused before any byte goes out.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let store = Store::in_memory().unwrap();
    let approvals = Arc::new(itda_policy::Approvals::open(store.clone()).unwrap());
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    let dir = std::env::temp_dir().join(format!("itda-runtime-unsent-{}", itda_types::util::id()));
    let log = Arc::new(AuditLog::new(dir.join("audit.jsonl")));
    let runtime = Runtime::new(Arc::new(Network::new(gate, true)), vault)
        .with_approvals(approvals.clone())
        .with_audit(log.clone());
    let spec = spec(&base);

    let error = runtime
        .call(&spec, call("create-item", &[("title", "hello")]))
        .await
        .unwrap_err();
    let id = error.extra["approval"].as_str().unwrap().to_owned();
    approvals.claim(&id).await.unwrap();
    let mut approved = call("create-item", &[("title", "hello")]);
    approved.approval = Some(id);
    let error = runtime.call(&spec, approved).await.unwrap_err();
    assert_eq!(error.public_code, "request_failed", "{error:?}");
    assert_eq!(error.extra["sent"], false);

    let records = log.records().unwrap();
    let last = records.last().unwrap();
    assert_eq!(
        (last.kind.as_str(), last.outcome.as_str()),
        ("call", "request_failed")
    );
    assert!(log.unfinished_changes().unwrap().is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_read_that_preflight_would_turn_into_a_change_is_not_sent() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let router = Router::new()
        .route(
            "/form",
            get(|| async { axum::Json(json!({"mode": "delete"})) }),
        )
        .route(
            "/items",
            get(move || async move {
                counted.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"items": []}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let value = json!({
        "spec_version": 1,
        "site": {"name": "fixture", "base_url": base},
        "operations": [
            {"name": "list-items", "method": "GET", "path": "/items", "effect": "read", "evidence": "doc",
             "preflight": [{"name": "form", "path": "/form", "extract": {"mode": "json:/mode"}}],
             "query": {"mode": {"type": "string", "default": "{{step.form.mode}}"}},
             "response": {"format": "json", "required_pointers": ["/items"]}}
        ]
    });
    let spec = itda_spec::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let (runtime, _) = runtime().await;
    let error = runtime
        .call(&spec, call("list-items", &[]))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "effect_changed");
    assert!(error.msg.contains("would change the site"), "{}", error.msg);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_unknown_input_names_the_nearest_declared_one() {
    let site = site().await;
    let (runtime, _) = runtime().await;
    let error = runtime
        .call(&spec(&site.base), call("list-items", &[("qu", "x")]))
        .await
        .unwrap_err();
    assert_eq!(error.msg, "Unknown input: qu");
    assert!(
        error.remedy.starts_with("Did you mean q?"),
        "{}",
        error.remedy
    );
    let error = runtime
        .call(&spec(&site.base), call("list-item", &[]))
        .await
        .unwrap_err();
    assert!(
        error.remedy.starts_with("Did you mean list-items?"),
        "{}",
        error.remedy
    );
    assert_eq!(site.hits.load(Ordering::SeqCst), 0);
}
