//! The preparation loop over JSON-RPC, the way an agent runs it, against a
//! local fixture: a docs page that names routes and links its OpenAPI
//! description, the API itself, a script that names more routes, and a
//! second site next to it. Nothing here contacts a real website.
#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
};
use itda_mcp::{Catalog, Mode, Server};
use itda_net::{Gate, Network};
use itda_runtime::Runtime;
use itda_store::Store;
use itda_vault::{MemoryStore, Vault};
use rmcp::ServiceExt;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};

/// Requests the fixture received, by method and path.
type Hits = Arc<Mutex<BTreeMap<String, usize>>>;

fn hit(hits: &Hits, key: &str) {
    *hits.lock().unwrap().entry(key.to_owned()).or_default() += 1;
}

fn count(hits: &Hits, key: &str) -> usize {
    hits.lock().unwrap().get(key).copied().unwrap_or(0)
}

const SECRET_WORDS: [&str; 4] = [
    "SECRET-RECORD-TITLE",
    "SECRET-DETAIL",
    "SECRET-COOKIE",
    "SECRET-CSRF",
];

async fn fixture() -> (String, Hits) {
    let hits: Hits = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let docs = format!(
        r#"<html><head><title>Fixture API</title><link rel="service-desc" href="/openapi.json">
<script src="/static/app.js"></script></head><body><h1>Fixture API</h1><h2>Items</h2>
<p><code>GET /api/items</code> lists the items. <code>GET /api/items/{{id}}</code> shows one.
<code>POST /api/items</code> adds one. <code>GET /api/broken</code> is kept for old clients.</p>
<form action="/docs/search" method="get"><input name="q"><input type="hidden" name="csrf_token" value="SECRET-CSRF"></form>
<a href="/docs/items">Items</a><a href="{base}/docs/auth">Auth</a></body></html>"#
    );
    let openapi = json!({
        "openapi": "3.0.0",
        "info": {"title": "Fixture", "version": "1"},
        "servers": [{"url": format!("{base}/api")}],
        "components": {"securitySchemes": {"oauth": {"type": "oauth2", "flows": {}}}},
        "paths": {
            "/items": {"get": {"responses": {"200": {"description": "ok"}}}, "post": {"responses": {"201": {"description": "ok"}}}},
            "/items/{id}": {"get": {"parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "integer"}}], "responses": {"200": {"description": "ok"}}}},
            "/admin": {"get": {"security": [{"oauth": []}], "responses": {"200": {"description": "ok"}}}}
        }
    });
    let router = Router::new()
        .route(
            "/",
            get(|| async { Html("<html><head><title>Fixture</title></head><body><a href='/docs'>Docs</a></body></html>") }),
        )
        .route(
            "/docs",
            get(move |State(hits): State<Hits>| {
                let docs = docs.clone();
                async move {
                    hit(&hits, "GET /docs");
                    let mut headers = HeaderMap::new();
                    headers.insert("set-cookie", "sid=SECRET-COOKIE".parse().unwrap());
                    (headers, Html(docs))
                }
            }),
        )
        .route(
            "/openapi.json",
            get(move |State(hits): State<Hits>| {
                let openapi = openapi.clone();
                async move {
                    hit(&hits, "GET /openapi.json");
                    axum::Json(openapi)
                }
            }),
        )
        .route(
            "/api/items",
            get(|State(hits): State<Hits>| async move {
                hit(&hits, "GET /api/items");
                axum::Json(json!({"items": [{"id": 7, "title": "SECRET-RECORD-TITLE"}], "total": 1}))
            })
            .post(|State(hits): State<Hits>| async move {
                hit(&hits, "POST /api/items");
                StatusCode::CREATED
            }),
        )
        .route(
            "/api/items/{id}",
            get(|State(hits): State<Hits>, Path(id): Path<String>| async move {
                hit(&hits, &format!("GET /api/items/{id}"));
                if id == "7" {
                    axum::Json(json!({"id": 7, "title": "SECRET-DETAIL", "body": "SECRET-DETAIL"}))
                        .into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/api/broken",
            get(|State(hits): State<Hits>| async move {
                hit(&hits, "GET /api/broken");
                axum::Json(json!({"error": "unauthorized"}))
            }),
        )
        .route(
            "/api/feed",
            get(|State(hits): State<Hits>| async move {
                hit(&hits, "GET /api/feed");
                axum::Json(json!({"entries": [{"n": 1}]}))
            }),
        )
        .route(
            "/api/noise",
            get(|State(hits): State<Hits>| async move {
                hit(&hits, "GET /api/noise");
                Html("<html><title>Not here</title></html>")
            }),
        )
        .route(
            "/static/app.js",
            get(|State(hits): State<Hits>| async move {
                hit(&hits, "GET /static/app.js");
                r#"fetch("/api/items"); fetch("/api/feed"); fetch("/api/noise"); fetch("/api/items", {method: "POST"});"#
            }),
        )
        .with_state(hits.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, hits)
}

/// A second site on its own address: a page with a title and little else.
async fn other_site() -> String {
    let router = Router::new().route(
        "/",
        get(|| async {
            Html("<html><head><title>Other</title></head><body><h1>Other</h1></body></html>")
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

struct Managed {
    server: Server,
    home: std::path::PathBuf,
    kb: Arc<itda_kb::Kb>,
}

fn managed() -> Managed {
    let home = std::env::temp_dir().join(format!("itda-prepare-{}", itda_types::util::id()));
    std::fs::create_dir_all(home.join("specs")).unwrap();
    let store = Store::in_memory().unwrap();
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let net = Arc::new(Network::new(gate, true));
    let vault = Arc::new(Vault::open(store.clone(), Arc::new(MemoryStore::default())).unwrap());
    let approvals = Arc::new(itda_policy::Approvals::open(store.clone()).unwrap());
    let kb = Arc::new(itda_kb::Kb::open(store).unwrap());
    let audit = Arc::new(itda_runtime::audit::AuditLog::new(home.join("audit.jsonl")));
    let runtime = Arc::new(
        Runtime::new(net.clone(), vault.clone())
            .with_approvals(approvals.clone())
            .with_audit(audit.clone())
            .with_kb(kb.recorder()),
    );
    let context = Arc::new(itda_setup::Context {
        home: home.clone(),
        net,
        vault,
        approvals,
        audit,
        runtime: runtime.clone(),
        kb: kb.clone(),
        lang: "en".into(),
    });
    let server =
        Server::new(runtime, Catalog::new(Vec::new()), Some(Mode::Direct)).with_setup(context);
    Managed { server, home, kb }
}

struct Client {
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    writer: WriteHalf<DuplexStream>,
    next: u64,
    /// Every result the agent saw, for the check that no value leaked.
    seen: String,
}

impl Client {
    async fn start(server: Server) -> Self {
        let (ours, theirs) = tokio::io::duplex(1 << 22);
        tokio::spawn(async move {
            let running = server.serve(tokio::io::split(theirs)).await.unwrap();
            running.waiting().await.unwrap();
        });
        let (reader, writer) = tokio::io::split(ours);
        let mut client = Self {
            lines: BufReader::new(reader).lines(),
            writer,
            next: 1,
            seen: String::new(),
        };
        client
            .request(
                "initialize",
                json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
            )
            .await;
        let mut line = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
        line.push('\n');
        client.writer.write_all(line.as_bytes()).await.unwrap();
        client
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await.unwrap();
        loop {
            let line = self.lines.next_line().await.unwrap().unwrap();
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == id {
                self.seen.push_str(&line);
                return message["result"].clone();
            }
        }
    }

    /// One `itda_prepare` call: its structured result.
    async fn prepare(&mut self, arguments: Value) -> Value {
        let result = self
            .request(
                "tools/call",
                json!({"name": "itda_prepare", "arguments": arguments}),
            )
            .await;
        let structured = result["structuredContent"].clone();
        assert!(
            structured.to_string().len() <= itda_mcp::RESULT_LIMIT,
            "{}",
            structured.to_string().len()
        );
        structured
    }
}

fn names(list: &Value, key: &str) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v[key].as_str().map(str::to_owned))
        .collect()
}

fn judgment<'a>(proposed: &'a Value, op: &str) -> &'a Value {
    proposed["judgments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["op"] == op)
        .unwrap_or_else(|| panic!("no judgment for {op}: {proposed}"))
}

#[tokio::test]
async fn an_agent_prepares_a_site_from_its_docs_and_itda_keeps_what_evidence_carries() {
    let (base, hits) = fixture().await;
    let other = other_site().await;
    let Managed { server, home, kb } = managed();
    let mut client = Client::start(server).await;

    let tools = client.request("tools/list", json!({})).await;
    assert!(
        names(&tools["tools"], "name").contains(&"itda_prepare".to_owned()),
        "{tools}"
    );

    let started = client
        .prepare(json!({"op": "start", "url": format!("{base}/docs"), "name": "fixture"}))
        .await;
    assert_eq!(started["status"], "ok", "{started}");
    let session = started["session"].as_str().unwrap().to_owned();
    assert_eq!(started["site"], "fixture");
    assert_eq!(started["environment"]["installed"], Value::Null);
    let again = client
        .prepare(json!({"op": "start", "url": format!("{base}/docs/items"), "name": "fixture"}))
        .await;
    assert_eq!(again["session"], started["session"]);
    assert_eq!(again["feedback"][0]["kind"], "already");

    // web.read stays on the site and the hosts it links as documentation.
    for (url, code) in [
        ("https://example.org/".to_owned(), "outside_site"),
        ("http://10.0.0.1/".to_owned(), "private_address"),
        (format!("{other}/"), "private_address"),
    ] {
        let refused = client
            .prepare(json!({"op": "read", "session": session, "url": url}))
            .await;
        assert_eq!(refused["status"], code, "{url}: {refused}");
    }

    let recon = client
        .prepare(json!({"op": "recon", "session": session}))
        .await;
    assert_eq!(recon["status"], "ok", "{recon}");
    assert_eq!(recon["entry"]["kind"], "html");
    assert_eq!(recon["description"]["kind"], "openapi");
    let routes: Vec<String> = recon["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r["method"].as_str().unwrap(),
                r["path"].as_str().unwrap()
            )
        })
        .collect();
    for route in [
        "GET /api/items",
        "GET /api/items/{id}",
        "POST /api/items",
        "GET /items",
    ] {
        assert!(routes.contains(&route.to_owned()), "{route} in {routes:?}");
    }
    assert!(
        !recon["candidates"].as_array().unwrap().is_empty(),
        "{recon}"
    );
    assert_eq!(
        count(&hits, "GET /static/app.js"),
        0,
        "bundles are off by default"
    );

    // Opt-in bundle mining: only a route that answers with data is evidence.
    let mined = client
        .prepare(json!({"op": "recon", "session": session, "bundles": true}))
        .await;
    let bundles = &mined["bundles"];
    assert_eq!(
        names(&bundles["answered"], "path"),
        ["/api/feed"],
        "{mined}"
    );
    assert_eq!(bundles["noise"], 1, "{mined}");
    assert_eq!(count(&hits, "POST /api/items"), 0);

    let spec = json!({
        "site": {"name": "anything", "title": "Fixture", "base_url": format!("{base}/api")},
        "operations": [
            {"name": "list-items", "desc": "List the items", "method": "GET", "path": "/items", "effect": "read",
             "response": {"format": "json"}},
            {"name": "get-item", "desc": "Show one item", "method": "GET", "path": "/items/{id}", "effect": "read",
             "params": {"id": {"type": "int"}}, "response": {"format": "json"}},
            {"name": "create-item", "desc": "Add an item", "method": "POST", "path": "/items", "effect": "write",
             "body": {"json": {"title": {"type": "string", "required": true}}}},
            {"name": "made-up", "desc": "Something no page names", "method": "GET", "path": "/secret-admin",
             "effect": "read", "response": {"format": "json"}},
            {"name": "elsewhere", "desc": "The same list elsewhere", "method": "GET", "path": "/items",
             "base_url": format!("{other}/api"), "effect": "read", "response": {"format": "json"}},
            {"name": "admin", "desc": "Admin", "method": "GET", "path": "/admin", "effect": "read",
             "response": {"format": "json"}},
            {"name": "broken", "desc": "The old endpoint", "method": "GET", "path": "/broken", "effect": "read",
             "response": {"format": "json"}},
            {"name": "feed", "desc": "The feed", "method": "GET", "path": "/feed", "effect": "read",
             "response": {"format": "json"}}
        ]
    });
    let first_route = recon["routes"][0]["id"].as_str().unwrap().to_owned();
    let proposed = client
        .prepare(json!({
            "op": "propose", "session": session, "spec": spec,
            "understand": {"workflows": [{"id": "browse", "title": "Browse items", "benefit": "see what is there",
                "steps": ["list the items", "open one"], "success": "one item shown"}]},
            "self_qa": [
                {"lens": "contract", "question": "is the list documented?", "evidence_ids": [first_route], "answer": "yes", "action": "keep"},
                {"lens": "premise", "question": "made up?", "evidence_ids": ["e999"], "answer": "no", "action": "keep"}
            ]
        }))
        .await;
    assert_eq!(proposed["status"], "ok", "{proposed}");
    for kept in ["list-items", "get-item", "broken", "feed"] {
        assert_eq!(
            judgment(&proposed, kept)["verdict"],
            "kept",
            "{kept}: {proposed}"
        );
    }
    let create = judgment(&proposed, "create-item");
    assert_eq!(
        (create["verdict"].as_str(), create["effect"].as_str()),
        (Some("kept"), Some("change"))
    );
    assert_eq!(create["confirm_with"], "list-items");
    assert!(
        judgment(&proposed, "made-up")["reason"]
            .as_str()
            .unwrap()
            .contains("no evidence")
    );
    assert!(
        judgment(&proposed, "elsewhere")["reason"]
            .as_str()
            .unwrap()
            .contains("not a server")
    );
    assert!(
        judgment(&proposed, "admin")["reason"]
            .as_str()
            .unwrap()
            .contains("sign-in itda cannot use")
    );
    assert_eq!(proposed["self_qa"]["accepted"], 1);
    assert!(
        proposed["self_qa"]["refused"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("e999")
    );
    assert_eq!(proposed["understand"]["workflows"], 1);

    // The same proposal is judged once.
    let same = client
        .prepare(json!({"op": "propose", "session": session, "spec": spec,
            "understand": {"workflows": [{"id": "browse", "title": "Browse items", "benefit": "see what is there",
                "steps": ["list the items", "open one"], "success": "one item shown"}]},
            "self_qa": [
                {"lens": "contract", "question": "is the list documented?", "evidence_ids": [first_route], "answer": "yes", "action": "keep"},
                {"lens": "premise", "question": "made up?", "evidence_ids": ["e999"], "answer": "no", "action": "keep"}
            ]}))
        .await;
    assert_eq!(same["already"], true, "{same}");

    // A list or an object is no input.
    let not_scalar = client
        .prepare(json!({"op": "verify", "session": session, "checks": [
            {"action": "list-items", "inputs": {}},
            {"action": "get-item", "inputs": {"id": "$result:list-items:/items"}}
        ]}))
        .await;
    let skipped = not_scalar["skipped"].as_array().unwrap();
    assert!(
        skipped.iter().any(|s| s["action"] == "get-item"
            && s["why"].as_str().unwrap().contains("a list or an object")),
        "{not_scalar}"
    );
    let listed = count(&hits, "GET /api/items");
    assert_eq!(listed, 1, "the list was tried once");
    // An input the read does not declare is refused before anything is
    // built, so it cannot turn the read into a change either.
    let odd = client
        .prepare(json!({"op": "verify", "session": session, "checks": [
            {"action": "feed", "inputs": {"action": "delete"}}
        ]}))
        .await;
    assert!(
        odd["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["action"] == "feed" && s["why"].as_str().unwrap().contains("not an input")),
        "{odd}"
    );

    let verified = client
        .prepare(json!({"op": "verify", "session": session, "checks": [
            {"action": "get-item", "inputs": {"id": "$result:list-items:/items/0/id"}}
        ]}))
        .await;
    assert_eq!(verified["status"], "ok", "{verified}");
    let results = verified["results"].as_array().unwrap();
    let result = |action: &str| {
        results
            .iter()
            .find(|r| r["action"] == action)
            .unwrap_or_else(|| panic!("no result for {action}: {verified}"))
    };
    assert_eq!(result("list-items")["already"], true, "{verified}");
    assert_eq!(
        count(&hits, "GET /api/items"),
        1,
        "a read with the same inputs is not sent again"
    );
    assert_eq!(result("get-item")["answered"], true, "{verified}");
    assert_eq!(
        count(&hits, "GET /api/items/7"),
        1,
        "the chain carried the list's first id"
    );
    assert_eq!(result("broken")["answered"], false);
    assert_eq!(result("feed")["answered"], true);
    assert_eq!(result("get-item")["shape"]["title"], "string");
    assert!(
        verified["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["action"] == "create-item"),
        "{verified}"
    );
    assert_eq!(count(&hits, "POST /api/items"), 0, "a change is never sent");
    assert_eq!(
        not_scalar["results"][0]["pointers_added"],
        json!(["/items", "/total"])
    );
    // The budget counts what went out: list, broken, feed, then one item.
    assert_eq!(
        verified["budgets"]["verify_requests"]["used"], 4,
        "{verified}"
    );

    let reviewed = client
        .prepare(json!({"op": "review", "session": session, "workflows": [
            {"id": "browse", "steps": [{"step": "list", "ops": ["list-items"]}, {"step": "open", "ops": ["get-item"]}]}
        ]}))
        .await;
    assert_eq!(reviewed["uncovered"], 0, "{reviewed}");
    assert!(reviewed["coverage"][1]["covered"].as_bool().unwrap());

    // A repair round: the read that answers with an error goes.
    let mut repaired = spec.clone();
    repaired["operations"].as_array_mut().unwrap().retain(|op| {
        !["broken", "made-up", "elsewhere", "admin"].contains(&op["name"].as_str().unwrap())
    });
    let round = client
        .prepare(json!({"op": "propose", "session": session, "spec": repaired}))
        .await;
    assert_eq!(round["round"], 2, "{round}");
    assert_eq!(round["budgets"]["repair_rounds"]["used"], 1);

    let installed = client
        .prepare(json!({"op": "install", "session": session}))
        .await;
    assert_eq!(installed["status"], "ok", "{installed}");
    assert_eq!(installed["site"], "fixture");
    let actions = names(&installed["actions"], "action");
    for action in ["list-items", "get-item", "create-item", "feed"] {
        assert!(
            actions.contains(&action.to_owned()),
            "{action}: {installed}"
        );
    }
    assert_eq!(
        count(&hits, "POST /api/items"),
        0,
        "installing never sends a change"
    );
    let yaml = std::fs::read_to_string(home.join("specs/fixture/spec.yaml")).unwrap();
    assert!(yaml.contains("/items"), "{yaml}");
    assert!(!yaml.contains("secret-admin"), "{yaml}");
    let tools = client.request("tools/list", json!({})).await;
    assert!(
        names(&tools["tools"], "name").contains(&"fixture.list-items".to_owned()),
        "{tools}"
    );
    let spec = itda_spec::parse(yaml.as_bytes()).unwrap();
    let list = spec.op("list-items").unwrap();
    assert_eq!(
        list.response.as_ref().unwrap().required_pointers,
        ["/items", "/total"]
    );

    // Nothing the site answered, and no secret, reached the agent or the
    // knowledge base.
    let dump = kb.dump().await.unwrap();
    for word in SECRET_WORDS {
        assert!(!client.seen.contains(word), "{word} reached the agent");
        assert!(!dump.contains(word), "{word} in the knowledge base");
    }
    // The preparation's tries are on record, but not held against the
    // definition.
    assert!(dump.contains("prepare"), "{dump}");
    let gaps = client
        .request(
            "tools/call",
            json!({"name": "itda_manage", "arguments": {"action": "fsck", "site": "fixture"}}),
        )
        .await;
    let rules = gaps["structuredContent"]["sites"][0]["gaps"].to_string();
    assert!(!rules.contains("finding_without_evidence"), "{rules}");
    assert!(!rules.contains("read_failing"), "{rules}");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test]
async fn the_home_page_alone_is_refused_and_budgets_end_the_loop() {
    let (base, _hits) = fixture().await;
    let Managed { server, home, .. } = managed();
    let mut client = Client::start(server).await;
    let started = client
        .prepare(json!({"op": "start", "url": format!("{base}/docs"), "name": "bare"}))
        .await;
    let session = started["session"].as_str().unwrap().to_owned();
    client
        .prepare(json!({"op": "read", "session": session, "url": format!("{base}/docs")}))
        .await;
    let home_only = client
        .prepare(json!({"op": "propose", "session": session, "spec": {
            "site": {"base_url": base},
            "operations": [{"name": "home", "method": "GET", "path": "/docs", "effect": "read", "response": {"format": "text"}}]
        }}))
        .await;
    assert_eq!(
        judgment(&home_only, "home")["verdict"],
        "kept",
        "{home_only}"
    );
    assert!(home_only["no_actions"].is_null());
    // A text answer holds no records, yet answers as declared: it covers a
    // step in review.
    let text = client
        .prepare(json!({"op": "verify", "session": session}))
        .await;
    assert_eq!(text["results"][0]["status"], "text", "{text}");
    let reviewed = client
        .prepare(json!({"op": "review", "session": session, "workflows": [
            {"id": "read", "steps": [{"step": "read the docs", "ops": ["home"]}]}
        ]}))
        .await;
    assert_eq!(reviewed["uncovered"], 0, "{reviewed}");

    // A page read whose selector picks several elements in a record fails;
    // the result says where, in counts, measured on the page read before.
    let list = client
        .prepare(json!({"op": "propose", "session": session, "spec": {
            "site": {"base_url": base},
            "operations": [{"name": "routes", "method": "GET", "path": "/docs", "effect": "read",
                "response": {"format": "html", "html": {"items": "p", "fields": {"route": {"selector": "code", "required": true}}}}}]
        }}))
        .await;
    assert_eq!(judgment(&list, "routes")["verdict"], "kept", "{list}");
    let tried = client
        .prepare(json!({"op": "verify", "session": session}))
        .await;
    let result = &tried["results"][0];
    assert_eq!(result["status"], "response_mismatch", "{tried}");
    assert_eq!(result["diagnosis"]["items_matched"], 1, "{tried}");
    assert_eq!(
        result["diagnosis"]["fields"]["route"]["found_several"], 1,
        "{tried}"
    );
    assert_eq!(result["diagnosis"]["measured_on"], "/docs");
    assert!(!tried.to_string().contains("SECRET"), "{tried}");

    // A read tried with inputs and not named again is not sent again.
    client
        .prepare(json!({"op": "propose", "session": session, "spec": {
            "site": {"base_url": base},
            "operations": [{"name": "search", "method": "GET", "path": "/docs/search", "effect": "read",
                "query": {"q": {"type": "string"}}, "response": {"format": "json"}}]
        }}))
        .await;
    let first = client
        .prepare(json!({"op": "verify", "session": session, "checks": [
            {"action": "search", "inputs": {"q": "items"}}
        ]}))
        .await;
    assert_eq!(first["results"][0]["sent"], 1, "{first}");
    let again = client
        .prepare(json!({"op": "verify", "session": session}))
        .await;
    assert_eq!(again["results"][0]["already"], true, "{again}");
    assert_eq!(
        again["budgets"]["verify_requests"]["used"],
        first["budgets"]["verify_requests"]["used"]
    );
    client
        .prepare(json!({"op": "read", "session": session, "url": format!("{base}/")}))
        .await;
    let root_only = client
        .prepare(json!({"op": "propose", "session": session, "spec": {
            "site": {"base_url": base},
            "operations": [{"name": "home", "method": "GET", "path": "/", "effect": "read", "response": {"format": "text"}}]
        }}))
        .await;
    assert!(root_only["no_actions"].is_string(), "{root_only}");
    let refused = client
        .prepare(json!({"op": "install", "session": session}))
        .await;
    assert_eq!(refused["status"], "no_actions", "{refused}");
    assert!(!home.join("specs/bare").exists());

    // Every session has a call budget; past it, nothing more runs.
    let mut last = Value::Null;
    for _ in 0..itda_setup::prepare::CALLS {
        last = client
            .prepare(json!({"op": "search", "session": session, "q": "items"}))
            .await;
        if last["status"] == "budget_exhausted" {
            break;
        }
    }
    assert_eq!(last["status"], "budget_exhausted", "{last}");
    assert_eq!(last["budget"], "tool calls");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test]
async fn a_server_for_one_site_prepares_no_other() {
    let Managed { server, home, .. } = managed();
    let mut client = Client::start(server.with_site("fixture")).await;
    let refused = client
        .prepare(json!({"op": "start", "url": "https://example.org/", "name": "fixture"}))
        .await;
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("never sets other sites up"),
        "{refused}"
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test]
async fn setup_that_keeps_one_read_points_at_prepare() {
    let other = other_site().await;
    let Managed { server, home, .. } = managed();
    let mut client = Client::start(server).await;
    let done = client
        .request(
            "tools/call",
            json!({"name": "itda_setup", "arguments": {"url": format!("{other}/"), "name": "other"}}),
        )
        .await;
    let structured = &done["structuredContent"];
    assert_eq!(structured["status"], "ok", "{done}");
    assert_eq!(structured["remedy"]["tool"], "itda_prepare", "{structured}");
    assert_eq!(structured["remedy"]["arguments"]["op"], "start");
    std::fs::remove_dir_all(home).ok();
}
