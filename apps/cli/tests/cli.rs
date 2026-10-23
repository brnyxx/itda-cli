//! Runs the built `itda` binary against a synthetic local server and a
//! temporary data directory. Nothing here contacts a real website or the
//! system keyring.
#![allow(clippy::unwrap_used)]
use axum::{Router, http::HeaderMap, routing::get};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn home() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("itda-cli-{}", itda_types::util::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn install(home: &Path, base: &str) {
    let dir = home.join("specs/fixture");
    std::fs::create_dir_all(&dir).unwrap();
    let spec = format!(
        "spec_version: 1
site:
  name: fixture
  base_url: {base}
auth:
  - name: key
    kind: header
    header: X-Key
    value_from: store:fixture/api-key
operations:
  - name: list-items
    desc: List items
    method: GET
    path: /items
    effect: read
    evidence: doc
    response: {{format: json, required_pointers: [/items]}}
  - name: create-item
    method: POST
    path: /items
    effect: write
    evidence: doc
  - name: me
    method: GET
    path: /me
    auth: key
    effect: read
    evidence: doc
  - name: get-item
    desc: Show one item
    method: GET
    path: /items/{{id}}
    params:
      id: {{type: int}}
    effect: read
    evidence: doc
  - name: denied
    method: GET
    path: /denied
    effect: read
    evidence: doc
  - name: gone
    method: GET
    path: /gone
    effect: read
    evidence: doc
"
    );
    std::fs::write(dir.join("spec.yaml"), spec).unwrap();
}

/// A second site whose one read hands out a cursor.
fn install_paged(home: &Path, base: &str) {
    let dir = home.join("specs/paged");
    std::fs::create_dir_all(&dir).unwrap();
    let spec = format!(
        "spec_version: 1
site:
  name: paged
  base_url: {base}
operations:
  - name: pages
    desc: List the pages
    method: GET
    path: /pages
    effect: read
    evidence: doc
    query:
      cursor: {{type: string}}
    pagination: {{style: cursor, next: /next_cursor, send: {{query: cursor}}}}
    response: {{format: json, extract_pointer: /items}}
"
    );
    std::fs::write(dir.join("spec.yaml"), spec).unwrap();
}

async fn serve() -> String {
    let router = Router::new()
        .route(
            "/items",
            get(|| async { axum::Json(json!({"items": [{"title": "first"}]})) }),
        )
        .route(
            "/me",
            get(|headers: HeaderMap| async move {
                axum::Json(json!({"key_seen": headers.contains_key("x-key")}))
            }),
        )
        .route(
            "/pages",
            get(
                |axum::extract::Query(q): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| async move {
                    let from: usize = q
                        .get("cursor")
                        .map(|c| c.trim_start_matches("cur-").parse().unwrap())
                        .unwrap_or(0);
                    let items: Vec<Value> = (from..(from + 2).min(4))
                        .map(|i| json!({"id": i}))
                        .collect();
                    let next = (from + 2 < 4).then(|| format!("cur-{}", from + 2));
                    axum::Json(json!({"items": items, "next_cursor": next}))
                },
            ),
        )
        .route(
            "/denied",
            get(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    axum::Json(json!({"error": {"code": "permission_denied"}})),
                )
            }),
        )
        .route("/gone", get(|| async { axum::http::StatusCode::GONE }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

async fn itda(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_itda"));
    command
        .args(args)
        .env("ITDA_HOME", home)
        .env("ITDA_ALLOW_LOCAL", "1")
        .env("ITDA_SECRET_STORE", "env")
        .env("ITDA_TRANSPORT", "http")
        .envs(env.iter().copied());
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_an_unknown_transport_before_sending() {
    let home = home();
    install(&home, "http://127.0.0.1:9");
    let result = itda(
        &home,
        &["call", "fixture.list-items", "--json"],
        &[("ITDA_TRANSPORT", "unknown")],
    )
    .await;
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(stdout_json(&result)["status"], "bad_request");
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unresolved_adaptive_read_does_not_lock_other_actions() {
    let home = home();
    let router = Router::new()
        .route(
            "/items",
            get(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    axum::response::Html("<title>Just a moment</title>"),
                )
            }),
        )
        .route(
            "/items/{id}",
            get(|| async { axum::Json(json!({"id": 1})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    install(&home, &format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let missing_browser = home.join("missing-browser");
    let env = [
        ("ITDA_TRANSPORT", "auto"),
        ("ITDA_BROWSER", missing_browser.to_str().unwrap()),
    ];
    let failed = itda(&home, &["call", "fixture.list-items", "--json"], &env).await;
    assert_eq!(failed.status.code(), Some(12), "{failed:?}");
    assert_eq!(stdout_json(&failed)["paused"], false);
    let available = itda(
        &home,
        &["call", "fixture.get-item", "--id", "1", "--json"],
        &env,
    )
    .await;
    assert!(available.status.success(), "{available:?}");
    assert_eq!(stdout_json(&available)["data"]["id"], 1);
    let stopped = itda(&home, &["__browser-stop"], &env).await;
    assert!(stopped.status.success(), "{stopped:?}");
    server.abort();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_installed_sites() {
    let home = home();
    let empty = itda(&home, &["ls"], &[]).await;
    assert!(empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stdout).contains("No sites installed"));

    install(&home, "http://127.0.0.1:9");
    let listed = stdout_json(&itda(&home, &["ls", "--json"], &[]).await);
    assert_eq!(listed[0]["site"], "fixture");
    let effects: Vec<&str> = listed[0]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["effect"].as_str().unwrap())
        .collect();
    assert_eq!(effects, ["read", "write", "read", "read", "read", "read"]);
    assert_eq!(
        listed[0]["actions"][3]["example"],
        "itda call fixture.get-item --id <int> --json"
    );
    std::fs::remove_dir_all(home).ok();
}
#[tokio::test(flavor = "multi_thread")]
async fn describe_shows_inputs_and_a_command_to_copy() {
    let home = home();
    install(&home, "http://127.0.0.1:9");
    let english = [("ITDA_LANG", "en")];
    let text =
        String::from_utf8_lossy(&itda(&home, &["describe", "fixture"], &english).await.stdout)
            .into_owned();
    assert!(text.contains("read   get-item"), "{text}");
    assert!(text.contains("inputs: id (int, required)"), "{text}");
    assert!(
        text.contains("example: itda call fixture.get-item --id <int> --json"),
        "{text}"
    );
    assert!(
        text.contains("example: itda call fixture.create-item --json"),
        "{text}"
    );
    assert!(text.contains("itda approve <id>"), "{text}");
    let korean = String::from_utf8_lossy(
        &itda(&home, &["describe", "fixture"], &[("ITDA_LANG", "ko")])
            .await
            .stdout,
    )
    .into_owned();
    assert!(korean.contains("입력: id (int, 필수)"), "{korean}");
    assert!(!korean.contains("example:"), "{korean}");

    let one = stdout_json(&itda(&home, &["describe", "fixture.get-item", "--json"], &[]).await);
    assert_eq!(one[0]["action"], "get-item");
    assert_eq!(one[0]["inputs"][0]["name"], "id");
    assert_eq!(one[0]["inputs"][0]["required"], true);
    assert_eq!(
        one[0]["example"],
        "itda call fixture.get-item --id <int> --json"
    );

    // A slip of the fingers gets the nearest name.
    let slip = itda(&home, &["call", "fixture.get-items", "--json"], &[]).await;
    assert_eq!(slip.status.code(), Some(2));
    assert!(
        stdout_json(&slip)["remedy"]
            .as_str()
            .unwrap()
            .starts_with("Did you mean get-item?")
    );
    let site = itda(&home, &["describe", "fixtures"], &[]).await;
    assert_eq!(site.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&site.stderr).contains("Did you mean fixture?"));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_names_what_the_site_answered() {
    let home = home();
    install(&home, &serve().await);
    let denied = itda(&home, &["call", "fixture.denied", "--json"], &[]).await;
    assert_eq!(denied.status.code(), Some(14));
    let envelope = stdout_json(&denied);
    assert_eq!(envelope["status"], "api_denied");
    assert_eq!(envelope["upstream_status"], 403);
    assert_eq!(envelope["provider_reason"], "permission_denied");
    let human = itda(&home, &["call", "fixture.denied"], &[]).await;
    assert!(
        String::from_utf8_lossy(&human.stderr)
            .contains("error[api_denied]: the site refused this request (permission_denied)")
    );

    // A gone address means the definition is out of date, not that the site is down.
    let checked = itda(
        &home,
        &["check", "fixture", "--json"],
        &[("ITDA_SECRET__FIXTURE__DEFAULT__API_KEY", "k")],
    )
    .await;
    assert_eq!(checked.status.code(), Some(11));
    let report = stdout_json(&checked);
    assert_eq!(report["verdict"], "site_changed");
    let result = |action: &str| {
        report["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["action"] == action)
            .unwrap()
            .clone()
    };
    assert_eq!(result("gone")["status"], "request_failed");
    assert_eq!(result("gone")["upstream_status"], 410);
    assert_eq!(result("gone")["detail"], "the site answered HTTP 410");
    assert_eq!(result("denied")["status"], "api_denied");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_and_doctor_show_a_change_without_an_outcome() {
    use itda_runtime::audit::{AuditLog, Event};
    let home = home();
    install(&home, "http://127.0.0.1:9");
    let log = AuditLog::new(home.join("audit.jsonl"));
    log.append(Event {
        kind: "change_started",
        site: "fixture".into(),
        action: "create-item".into(),
        effect: "write".into(),
        approval: "abc123".into(),
        outcome: "sending".into(),
        ..Event::default()
    })
    .unwrap();

    let audit = itda(&home, &["audit"], &[]).await;
    let text = String::from_utf8_lossy(&audit.stdout);
    assert!(
        text.contains(
            "1 change with an unknown outcome: the process stopped after the change was sent."
        ),
        "{text}"
    );
    assert!(
        text.contains("fixture.create-item (record 1, approval abc123)"),
        "{text}"
    );
    let doctor = itda(&home, &["doctor"], &[]).await;
    let text = String::from_utf8_lossy(&doctor.stdout);
    assert!(
        text.contains("changes          1 change with an unknown outcome"),
        "{text}"
    );
    assert!(text.contains("chain intact"), "{text}");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_lists_lost_answers_with_what_the_confirming_read_said() {
    use itda_runtime::audit::{AuditLog, Event};
    let home = home();
    install(&home, "http://127.0.0.1:9");
    let log = AuditLog::new(home.join("audit.jsonl"));
    let lost = |check: &str| Event {
        kind: "call",
        site: "fixture".into(),
        action: "create-item".into(),
        effect: "write".into(),
        approval: "abc123".into(),
        outcome: "action_outcome_unknown".into(),
        outcome_check: check.into(),
        confirm_read: if check.is_empty() {
            String::new()
        } else {
            "list-items".into()
        },
        ..Event::default()
    };
    log.append(lost("landed")).unwrap();
    log.append(lost("not_found")).unwrap();
    log.append(lost("")).unwrap();

    let audit = itda(&home, &["audit"], &[("ITDA_LANG", "en")]).await;
    let text = String::from_utf8_lossy(&audit.stdout);
    assert!(
        text.contains("3 change(s) whose answer was lost after sending:"),
        "{text}"
    );
    assert!(
        text.contains(
            "fixture.create-item (record 1, approval abc123): list-items showed it landed"
        ),
        "{text}"
    );
    assert!(
        text.contains("(record 2, approval abc123): list-items did not show it"),
        "{text}"
    );
    assert!(
        text.contains("(record 3, approval abc123): no confirming read; check the site"),
        "{text}"
    );
    // Only the records shown are summed up.
    let last = itda(&home, &["audit", "--limit", "1"], &[("ITDA_LANG", "en")]).await;
    let text = String::from_utf8_lossy(&last.stdout);
    assert!(text.contains("1 change(s) whose answer was lost"), "{text}");
    assert!(!text.contains("record 1,"), "{text}");
    std::fs::remove_dir_all(home).ok();
}

/// A site that keeps a POST and then drops the connection without an
/// answer, while its GET lists what it kept.
async fn serve_lossy() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let kept: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buffer = [0u8; 4096];
            let request = loop {
                let n = socket.read(&mut buffer).await.unwrap_or(0);
                if n == 0 {
                    break String::from_utf8_lossy(&raw).into_owned();
                }
                raw.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&raw).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break text;
                    }
                }
            };
            if request.starts_with("POST") {
                let body = request.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
                if let Ok(json) = serde_json::from_str::<Value>(body)
                    && let Some(title) = json["title"].as_str()
                {
                    kept.lock().unwrap().push(title.to_owned());
                }
                drop(socket);
                continue;
            }
            let items: Vec<Value> = kept
                .lock()
                .unwrap()
                .iter()
                .map(|title| json!({"title": title}))
                .collect();
            let body = json!({"items": items}).to_string();
            let answer = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(answer.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    base
}

fn install_confirming(home: &Path, base: &str) {
    let dir = home.join("specs/lossy");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("spec.yaml"),
        format!(
            "spec_version: 1
site: {{name: lossy, base_url: {base}}}
operations:
  - name: list-items
    desc: List items
    method: GET
    path: /items
    effect: read
    evidence: doc
    response: {{format: json, required_pointers: [/items]}}
  - name: create-item
    desc: Add an item
    method: POST
    path: /items
    effect: write
    evidence: doc
    body:
      json:
        title: {{type: string, required: true}}
    confirm:
      read: list-items
      match: {{pointer: /items/*/title, from: body.title}}
"
        ),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_names_the_read_that_confirms_a_change() {
    let home = home();
    install_confirming(&home, "http://127.0.0.1:9");
    let text = String::from_utf8_lossy(
        &itda(&home, &["describe", "lossy"], &[("ITDA_LANG", "en")])
            .await
            .stdout,
    )
    .into_owned();
    assert!(
        text.contains("after a lost answer: checks list-items for the new title"),
        "{text}"
    );
    assert_eq!(text.matches("after a lost answer").count(), 1, "{text}");
    let korean = String::from_utf8_lossy(
        &itda(&home, &["describe", "lossy"], &[("ITDA_LANG", "ko")])
            .await
            .stdout,
    )
    .into_owned();
    assert!(korean.contains("list-items에서 새 title 값을"), "{korean}");

    let rows = stdout_json(&itda(&home, &["describe", "lossy", "--json"], &[]).await);
    assert!(rows[0].get("confirm").is_none(), "{rows}");
    assert_eq!(
        rows[1]["confirm"],
        serde_json::json!({"read": "list-items", "match": {"pointer": "/items/*/title", "from": "body.title"}})
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_validate_checks_a_file_without_installing_it() {
    let home = home();
    install_confirming(&home, "http://127.0.0.1:9");
    let good = home.join("specs/lossy/spec.yaml");
    let english = [("ITDA_LANG", "en")];
    let valid = itda(
        &home,
        &["spec", "validate", good.to_str().unwrap()],
        &english,
    )
    .await;
    assert_eq!(valid.status.code(), Some(0));
    let said = String::from_utf8_lossy(&valid.stdout);
    assert!(
        said.contains("a valid definition of lossy (2 actions)"),
        "{said}"
    );
    let report = stdout_json(
        &itda(
            &home,
            &["spec", "validate", good.to_str().unwrap(), "--json"],
            &[],
        )
        .await,
    );
    assert_eq!(report["status"], "ok");
    assert_eq!(report["issues"], serde_json::json!([]));

    let bad = home.join("bad.yaml");
    std::fs::write(
        &bad,
        std::fs::read_to_string(&good)
            .unwrap()
            .replace("read: list-items", "read: list-things")
            .replace("http://127.0.0.1:9", "ftp://127.0.0.1:9"),
    )
    .unwrap();
    let invalid = itda(
        &home,
        &["spec", "validate", bad.to_str().unwrap()],
        &english,
    )
    .await;
    assert_eq!(invalid.status.code(), Some(2));
    let said = String::from_utf8_lossy(&invalid.stderr);
    assert!(said.contains("problem: site.base_url"), "{said}");
    assert!(said.contains("list-things"), "{said}");
    assert!(said.contains("itda would not install it"), "{said}");
    let report = stdout_json(
        &itda(
            &home,
            &["spec", "validate", bad.to_str().unwrap(), "--json"],
            &[],
        )
        .await,
    );
    assert_eq!(report["status"], "data_invalid");
    let fields: Vec<&str> = report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&"site.base_url"), "{report}");
    assert!(fields.iter().any(|f| f.contains("confirm")), "{report}");

    std::fs::write(&bad, "operations: [").unwrap();
    let unreadable = itda(
        &home,
        &["spec", "validate", bad.to_str().unwrap(), "--json"],
        &[],
    )
    .await;
    assert_eq!(unreadable.status.code(), Some(2));
    assert_eq!(stdout_json(&unreadable)["status"], "data_invalid");
    // Nothing was installed.
    assert!(!home.join("specs/bad").exists());
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_change_is_checked_against_the_read_it_names() {
    let home = home();
    let base = serve_lossy().await;
    install_confirming(&home, &base);
    let en = [("ITDA_LANG", "en")];

    let waiting = itda(
        &home,
        &["call", "lossy.create-item", "--title", "hi", "--json"],
        &en,
    )
    .await;
    let id = stdout_json(&waiting)["approval"]
        .as_str()
        .unwrap()
        .to_owned();
    let store =
        itda_policy::Approvals::open(itda_store::Store::open(&home.join("itda.db")).unwrap())
            .unwrap();
    store.claim(&id).await.unwrap();
    let lost = itda(
        &home,
        &[
            "call",
            "lossy.create-item",
            "--title",
            "hi",
            "--approval",
            &id,
            "--json",
        ],
        &en,
    )
    .await;
    assert_eq!(lost.status.code(), Some(22), "{lost:?}");
    let envelope = stdout_json(&lost);
    assert_eq!(envelope["status"], "action_outcome_unknown");
    assert_eq!(envelope["outcome_check"]["read"], "list-items");
    assert_eq!(envelope["outcome_check"]["result"], "landed");

    // The same change again: the person is told the first one landed, in
    // the approval review and in the agent's answer.
    let again = itda(&home, &["call", "lossy.create-item", "--title", "hi"], &en).await;
    assert_eq!(again.status.code(), Some(20), "{again:?}");
    let said = String::from_utf8_lossy(&again.stderr);
    assert!(said.contains("showed it landed"), "{said}");
    let pending = stdout_json(&itda(&home, &["approvals", "--json"], &en).await);
    let note = pending[0]["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == "note")
        .unwrap();
    assert!(
        note["value"]
            .as_str()
            .unwrap()
            .contains("list-items showed it landed"),
        "{note}"
    );
    let id = pending[0]["id"].as_str().unwrap().to_owned();
    store.claim(&id).await.unwrap();
    // Without --json the check is said in words, in the person's language.
    let lost = itda(
        &home,
        &[
            "call",
            "lossy.create-item",
            "--title",
            "hi",
            "--approval",
            &id,
        ],
        &en,
    )
    .await;
    assert_eq!(lost.status.code(), Some(22), "{lost:?}");
    let said = String::from_utf8_lossy(&lost.stderr);
    assert!(
        said.contains("The site did not answer, but list-items shows the change landed."),
        "{said}"
    );
    let audit = itda(&home, &["audit"], &en).await;
    let text = String::from_utf8_lossy(&audit.stdout);
    assert!(text.contains("list-items showed it landed"), "{text}");
    let verified = stdout_json(&itda(&home, &["audit", "--verify", "--json"], &en).await);
    assert_eq!(verified["status"], "ok");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_return_an_untrusted_envelope() {
    let home = home();
    install(&home, &serve().await);
    let output = itda(&home, &["call", "fixture.list-items", "--json"], &[]).await;
    assert!(output.status.success(), "{output:?}");
    let envelope = stdout_json(&output);
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["untrusted"], true);
    assert_eq!(envelope["data"]["items"][0]["title"], "first");

    let human = itda(&home, &["call", "fixture.list-items"], &[]).await;
    assert!(String::from_utf8_lossy(&human.stdout).contains("\"first\""));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_continues_with_next_and_refuses_other_handles() {
    let home = home();
    let base = serve().await;
    install(&home, &base);
    install_paged(&home, &base);
    let first = itda(&home, &["call", "paged.pages", "--json"], &[]).await;
    assert!(first.status.success(), "{first:?}");
    let first = stdout_json(&first);
    assert_eq!(first["data"][0]["id"], 0);
    let handle = first["meta"]["next"].as_str().unwrap().to_owned();
    assert!(handle.starts_with("ic_"), "{handle}");
    assert!(!first.to_string().contains("cur-"));

    let second = itda(
        &home,
        &["call", "paged.pages", "--next", &handle, "--json"],
        &[],
    )
    .await;
    assert!(second.status.success(), "{second:?}");
    let second = stdout_json(&second);
    assert_eq!(second["data"][0]["id"], 2);
    assert!(second["meta"].get("next").is_none(), "{second}");

    // For people: the answer, then where to go on.
    let human = itda(&home, &["call", "paged.pages"], &[]).await;
    assert!(String::from_utf8_lossy(&human.stderr).contains("--next ic_"));

    let other = itda(
        &home,
        &["call", "fixture.list-items", "--next", &handle, "--json"],
        &[],
    )
    .await;
    assert_eq!(other.status.code(), Some(2));
    let made_up = itda(
        &home,
        &["call", "paged.pages", "--next", "cur-2", "--json"],
        &[],
    )
    .await;
    assert_eq!(made_up.status.code(), Some(23));
    assert_eq!(stdout_json(&made_up)["status"], "pagination_invalid");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_codes_follow_the_contract() {
    let home = home();
    install(&home, &serve().await);

    let write = itda(&home, &["call", "fixture.create-item", "--json"], &[]).await;
    assert_eq!(write.status.code(), Some(20));
    assert_eq!(stdout_json(&write)["status"], "approval_required");

    let signed_out = itda(&home, &["call", "fixture.me"], &[]).await;
    assert_eq!(signed_out.status.code(), Some(10));
    assert!(String::from_utf8_lossy(&signed_out.stderr).contains("error[auth_required]"));

    let unknown = itda(&home, &["call", "nowhere.read"], &[]).await;
    assert_eq!(unknown.status.code(), Some(2));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn environment_secrets_sign_requests() {
    let home = home();
    install(&home, &serve().await);
    let output = itda(
        &home,
        &["call", "fixture.me", "--json"],
        &[("ITDA_SECRET__FIXTURE__DEFAULT__API_KEY", "env-secret-value")],
    )
    .await;
    assert!(output.status.success(), "{output:?}");
    let envelope = stdout_json(&output);
    assert_eq!(envelope["data"]["key_seen"], true);
    assert_eq!(envelope["account"], "default");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("env-secret-value"));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn doctor_reports_the_setup() {
    let home = home();
    let output = itda(&home, &["doctor"], &[]).await;
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("data directory"));
    assert!(text.contains("ITDA_SECRET__"));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn feedback_prints_an_issue_link_without_the_data_directory() {
    let home = home();
    let output = itda(&home, &["feedback", "--no-open"], &[]).await;
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    let link = text.trim();
    assert!(
        link.starts_with("https://github.com/brnyxx/itda-cli/issues/new?"),
        "{link}"
    );
    let url = url::Url::parse(link).unwrap();
    let params: std::collections::BTreeMap<String, String> =
        url.query_pairs().into_owned().collect();
    assert_eq!(
        params["title"],
        format!(
            "itda {} on {}/{}: ",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    );
    assert!(
        params["body"].contains("data directory"),
        "{}",
        params["body"]
    );
    assert!(
        !params["body"].contains(&home.display().to_string()),
        "{}",
        params["body"]
    );
    assert!(!params["body"].contains("itda-cli-"), "{}", params["body"]);
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn the_agent_guide_and_skill_are_short_and_cover_the_rules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |file: &str| {
        std::fs::read_to_string(root.join(file))
            .unwrap()
            .replace("\r\n", "\n")
    };
    let skill = read("skills/itda/SKILL.md");
    let guide = read("AGENTS.md");
    assert!(
        skill.starts_with("---\nname: itda\ndescription: "),
        "{skill}"
    );
    assert!(skill.contains("AGENTS.md"), "{skill}");
    for text in [&skill, &guide] {
        assert!(text.lines().count() <= 120, "{}", text.lines().count());
        for phrase in [
            "itda ls",
            "itda approve <id>",
            "approval_pending",
            "api_denied",
            "action_outcome_unknown",
            "itda check <site>",
            "signed_out",
            "site_changed",
            "blocked",
            "unreachable",
            "itda setup <url>",
            "itda feedback",
        ] {
            assert!(text.contains(phrase), "{phrase}");
        }
    }
}

#[test]
fn the_sandbox_definition_is_valid() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sandbox/spec.yaml");
    let spec = itda_spec::parse(&std::fs::read(path).unwrap()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();
}

#[test]
fn the_bundled_sites_are_valid_and_named_after_their_files() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sites");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let spec = itda_spec::parse(&std::fs::read(&path).unwrap()).unwrap();
        itda_policy::validate_spec(&spec).unwrap();
        assert_eq!(
            Some(spec.site.name.as_str()),
            path.file_stem().and_then(|s| s.to_str())
        );
        count += 1;
    }
    assert!(count > 0);
}

#[tokio::test]
async fn mcp_serves_installed_actions_over_stdio() {
    use std::io::Write;
    let home = home();
    install(&home, "http://127.0.0.1:9");
    let mut child = Command::new(env!("CARGO_BIN_EXE_itda"))
        .arg("mcp")
        .env("ITDA_HOME", &home)
        .env("ITDA_SECRET_STORE", "env")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    ];
    {
        let stdin = child.stdin.as_mut().unwrap();
        for message in messages {
            writeln!(stdin, "{message}").unwrap();
        }
    }
    let stdout = child.stdout.take().unwrap();
    let lines = tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        std::io::BufReader::new(stdout)
            .lines()
            .take(2)
            .map(Result::unwrap)
            .collect::<Vec<_>>()
    })
    .await
    .unwrap();
    drop(child.stdin.take());
    child.wait().unwrap();

    // Every line on stdout is protocol.
    let replies: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "itda");
    let names: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "fixture.create-item",
            "fixture.denied",
            "fixture.get-item",
            "fixture.gone",
            "fixture.list-items",
            "fixture.me",
            "itda_setup",
            "itda_login",
            "itda_check",
            "itda_sites",
            "itda_manage",
        ]
    );
    assert_eq!(
        replies[0]["result"]["capabilities"]["tools"]["listChanged"],
        true
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test]
async fn mcp_install_dry_run_shows_the_change() {
    let home = home();
    let output = itda(
        &home,
        &["mcp", "install", "cursor", "--dry-run"],
        &[("HOME", home.to_str().unwrap())],
    )
    .await;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(".cursor/mcp.json"));
    assert!(text.contains("\"ITDA_ALLOW_LOCAL\": \"1\""));
    assert!(!home.join(".cursor").exists());

    let unknown = itda(&home, &["mcp", "install", "notepad"], &[]).await;
    assert_eq!(unknown.status.code(), Some(2));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_wait_for_a_person_to_approve_them() {
    let home = home();
    install(&home, &serve().await);
    let write = itda(&home, &["call", "fixture.create-item", "--json"], &[]).await;
    let id = stdout_json(&write)["approval"].as_str().unwrap().to_owned();

    let pending = stdout_json(&itda(&home, &["approvals", "--json"], &[]).await);
    assert_eq!(pending[0]["id"], id.as_str());
    assert_eq!(pending[0]["action_id"], "create-item");

    // Without a terminal nobody can answer the question, so nothing is approved.
    let piped = itda(&home, &["approve", &id], &[]).await;
    assert_eq!(piped.status.code(), Some(2));
    let shown = String::from_utf8_lossy(&piped.stdout);
    assert!(shown.contains("create-item wants to change"), "{shown}");
    assert!(shown.contains("POST"), "{shown}");
    // Before the person answers, the agent is told to wait, not to ask again.
    let early = itda(
        &home,
        &["call", "fixture.create-item", "--approval", &id, "--json"],
        &[],
    )
    .await;
    assert_eq!(early.status.code(), Some(20));
    let waiting = stdout_json(&early);
    assert_eq!(waiting["status"], "approval_pending");
    assert_eq!(waiting["approval"], id.as_str());
    assert_eq!(waiting["command"], format!("itda approve {id}"));
    let still = stdout_json(&itda(&home, &["approvals", "--json"], &[]).await);
    assert_eq!(still[0]["id"], id.as_str());

    assert!(itda(&home, &["reject", &id], &[]).await.status.success());
    let none = itda(&home, &["approvals"], &[]).await;
    assert!(String::from_utf8_lossy(&none.stdout).contains("No changes"));

    // Removing the site withdraws what was waiting for it.
    let again = itda(&home, &["call", "fixture.create-item", "--json"], &[]).await;
    assert_eq!(again.status.code(), Some(20));
    assert!(
        itda(&home, &["remove", "fixture"], &[])
            .await
            .status
            .success()
    );
    let gone = stdout_json(&itda(&home, &["approvals", "--json"], &[]).await);
    assert_eq!(gone, json!([]));

    // Every step is in the audit log, and the chain is intact.
    let records = stdout_json(&itda(&home, &["audit", "--json"], &[]).await);
    let kinds: Vec<(&str, &str)> = records
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["kind"].as_str().unwrap(), r["outcome"].as_str().unwrap()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("call", "approval_required"),
            ("call", "approval_pending"),
            ("rejected", "ok"),
            ("call", "approval_required"),
        ]
    );
    let verified = stdout_json(&itda(&home, &["audit", "--verify", "--json"], &[]).await);
    assert_eq!(verified["records"], 4);
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drafted_definition_runs_against_the_site() {
    let home = home();
    let base = serve().await;
    let document = json!({
        "openapi": "3.0.3",
        "info": {"title": "Fixture API", "version": "1"},
        "servers": [{"url": "https://fixture.example"}],
        "paths": {"/items": {"get": {
            "operationId": "listItems",
            "summary": "List items",
            "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {
                "type": "object", "required": ["items"], "properties": {"items": {"type": "array"}}
            }}}}}
        }}}
    });
    let file = home.join("openapi.json");
    std::fs::write(&file, document.to_string()).unwrap();

    let printed = itda(&home, &["draft", "openapi", file.to_str().unwrap()], &[]).await;
    assert!(printed.status.success(), "{printed:?}");
    assert!(String::from_utf8_lossy(&printed.stdout).contains("name: list-items"));
    assert!(String::from_utf8_lossy(&printed.stderr).contains("1 actions (1 reads, 0 changes)"));

    let installed = itda(
        &home,
        &[
            "draft",
            "openapi",
            file.to_str().unwrap(),
            "--base-url",
            &base,
            "--install",
        ],
        &[],
    )
    .await;
    assert!(installed.status.success(), "{installed:?}");
    let called = itda(&home, &["call", "fixture-api.list-items", "--json"], &[]).await;
    assert_eq!(stdout_json(&called)["data"]["items"][0]["title"], "first");

    let again = itda(
        &home,
        &["draft", "openapi", file.to_str().unwrap(), "--install"],
        &[],
    )
    .await;
    assert_eq!(again.status.code(), Some(2));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fetched_page_becomes_actions_that_run() {
    let home = home();
    let router = Router::new().route(
        "/board",
        get(|| async {
            axum::response::Html(
                "<title>Board</title><h1>Notices</h1>
                 <form action='/board/search' method='get'><input name='q' required></form>
                 <table><thead><tr><th>No</th><th>Title</th></tr></thead>
                 <tr><td>2</td><td><a href='/n/2'>Second</a></td></tr>
                 <tr><td>1</td><td><a href='/n/1'>First</a></td></tr></table>
                 <form action='/board' method='post'><input type='hidden' name='_csrf' value='tok-123'><input name='title'></form>",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let printed = itda(&home, &["draft", "page", &format!("{base}/board")], &[]).await;
    assert!(printed.status.success(), "{printed:?}");
    let yaml = String::from_utf8_lossy(&printed.stdout);
    assert!(yaml.contains("name: get-board"), "{yaml}");
    assert!(yaml.contains("name: search-board"), "{yaml}");
    assert!(yaml.contains("name: post-board"), "{yaml}");
    assert!(!yaml.contains("tok-123"));
    let said = String::from_utf8_lossy(&printed.stderr);
    assert!(said.contains("3 actions (2 reads, 1 changes)"), "{said}");
    assert!(
        !said.contains("API description"),
        "a site with no description says nothing about one: {said}"
    );

    let installed = itda(
        &home,
        &[
            "draft",
            "page",
            &format!("{base}/board"),
            "--name",
            "board",
            "--install",
        ],
        &[],
    )
    .await;
    assert!(installed.status.success(), "{installed:?}");
    let called = itda(&home, &["call", "board.get-board", "--json"], &[]).await;
    assert!(called.status.success(), "{called:?}");
    let data = &stdout_json(&called)["data"];
    assert_eq!(data["items"][0]["title"], "Second");
    assert_eq!(data["items"][1]["link"], format!("{base}/n/1"));

    let missing = itda(&home, &["draft", "page", &format!("{base}/nowhere")], &[]).await;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("HTTP 404"));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recorded_session_becomes_actions() {
    let home = home();
    let base = serve().await;
    let recording = json!({"log": {"entries": [{
        "request": {"method": "GET", "url": format!("{base}/items"), "headers": [
            {"name": "Authorization", "value": "Bearer recorded-token"}
        ]},
        "response": {"status": 200, "content": {
            "mimeType": "application/json",
            "text": "{\"items\": [{\"title\": \"recorded\"}]}"
        }}
    }]}});
    let file = home.join("session.har");
    std::fs::write(&file, recording.to_string()).unwrap();

    let installed = itda(
        &home,
        &[
            "draft",
            "har",
            file.to_str().unwrap(),
            "--name",
            "fixture",
            "--install",
        ],
        &[],
    )
    .await;
    assert!(installed.status.success(), "{installed:?}");
    let spec = std::fs::read_to_string(home.join("specs/fixture/spec.yaml")).unwrap();
    assert!(!spec.contains("recorded-token"));
    assert!(spec.contains("name: list-items"));
    // The copy kept for a refresh holds the shape of the recording, not its
    // values, and the refresh drafts the same actions from it.
    let kept = std::fs::read_to_string(home.join("specs/fixture/recording.har")).unwrap();
    assert!(!kept.contains("recorded-token"), "{kept}");
    assert!(kept.contains("Bearer <redacted>"), "{kept}");
    let sources: Value =
        serde_json::from_slice(&std::fs::read(home.join("specs/fixture/sources.json")).unwrap())
            .unwrap();
    assert_eq!(sources["recording"], "recording.har");
    let refreshed = itda(&home, &["refresh", "fixture", "-y"], &[("ITDA_LANG", "en")]).await;
    assert!(refreshed.status.success(), "{refreshed:?}");
    let said = String::from_utf8_lossy(&refreshed.stderr);
    assert!(said.contains("from its recording"), "{said}");
    assert!(said.contains("draft the same actions"), "{said}");

    let called = itda(&home, &["call", "fixture.list-items"], &[]).await;
    assert_eq!(called.status.code(), Some(10), "{called:?}");
    assert!(String::from_utf8_lossy(&called.stderr).contains("error[auth_required]"));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn check_runs_reads_that_need_no_input_and_reports_each() {
    let home = home();
    install(&home, &serve().await);

    let signed_out = itda(&home, &["check", "--json"], &[]).await;
    assert_eq!(signed_out.status.code(), Some(10));
    let report = stdout_json(&signed_out);
    assert_eq!(report["status"], "auth_required");
    let status = |action: &str| {
        report["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["action"] == action)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status("list-items"), "ok");
    assert_eq!(status("create-item"), "skipped");
    assert_eq!(status("me"), "auth_required");
    assert_eq!(report["verdict"], "signed_out");
    assert_eq!(report["sites"][0]["site"], "fixture");
    assert_eq!(report["sites"][0]["verdict"], "signed_out");

    let human = itda(&home, &["check", "fixture"], &[]).await;
    assert_eq!(human.status.code(), Some(10));
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("verdict  signed_out: sign in again with itda login fixture"),
        "{text}"
    );

    let signed_in = itda(
        &home,
        &["check", "fixture"],
        &[("ITDA_SECRET__FIXTURE__DEFAULT__API_KEY", "k")],
    )
    .await;
    // The gone address outranks the refused one; neither is a sign-in problem.
    assert_eq!(signed_in.status.code(), Some(11));
    let text = String::from_utf8_lossy(&signed_in.stdout);
    assert!(text.contains("verdict  site_changed: "), "{text}");
    assert!(text.contains("2 ok, 2 failed, 2 skipped."), "{text}");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn check_gives_each_site_a_verdict_and_a_changed_site_exits_11() {
    let home = home();
    let base = serve().await;
    install(&home, &base);
    let dir = home.join("specs/changed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("spec.yaml"),
        format!(
            "spec_version: 1
site:
  name: changed
  base_url: {base}
operations:
  - name: profile
    method: GET
    path: /me
    effect: read
    evidence: doc
    response: {{format: json, required_pointers: [/name]}}
"
        ),
    )
    .unwrap();

    let alone = itda(&home, &["check", "changed", "--json"], &[]).await;
    assert_eq!(alone.status.code(), Some(11));
    let report = stdout_json(&alone);
    assert_eq!(report["verdict"], "site_changed");
    assert_eq!(report["sites"][0]["verdict"], "site_changed");
    assert_eq!(report["results"][0]["status"], "response_mismatch");

    let human = itda(&home, &["check", "changed"], &[]).await;
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("verdict  site_changed: "), "{text}");

    // A signed-out account outranks a changed site when both are checked.
    let both = itda(&home, &["check", "--json"], &[]).await;
    assert_eq!(both.status.code(), Some(10));
    let report = stdout_json(&both);
    assert_eq!(report["verdict"], "signed_out");
    let verdict = |site: &str| {
        report["sites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["site"] == site)
            .unwrap()["verdict"]
            .clone()
    };
    assert_eq!(verdict("fixture"), "signed_out");
    assert_eq!(verdict("changed"), "site_changed");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn accounts_start_empty_and_logout_needs_one() {
    let home = home();
    install(&home, "http://127.0.0.1:9");
    let listed = itda(&home, &["accounts", "--json"], &[]).await;
    assert_eq!(listed.status.code(), Some(0));
    assert_eq!(stdout_json(&listed), json!([]));
    let logout = itda(&home, &["logout", "fixture"], &[]).await;
    assert_eq!(logout.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&logout.stderr).contains("not signed in"));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn add_installs_a_definition_and_remove_takes_it_away() {
    let home = home();
    let file = home.join("weather.yaml");
    std::fs::write(
        &file,
        "spec_version: 1
site: {name: weather, base_url: 'https://api.example.com'}
operations:
  - {name: forecast, method: GET, path: /forecast, effect: read, evidence: doc}
  - {name: delete-station, method: DELETE, path: /stations/1, effect: write, evidence: doc}
",
    )
    .unwrap();
    let added = itda(&home, &["add", file.to_str().unwrap()], &[]).await;
    assert_eq!(added.status.code(), Some(0));
    let review = String::from_utf8_lossy(&added.stderr);
    assert!(review.contains("https://api.example.com"), "{review}");
    assert!(review.contains("delete-station"), "{review}");
    let listed = itda(&home, &["ls", "weather", "--json"], &[]).await;
    assert_eq!(listed.status.code(), Some(0));
    let again = itda(&home, &["add", file.to_str().unwrap()], &[]).await;
    assert_eq!(again.status.code(), Some(2));
    let forced = itda(&home, &["add", file.to_str().unwrap(), "--force"], &[]).await;
    assert_eq!(forced.status.code(), Some(0));
    let removed = itda(&home, &["remove", "weather"], &[]).await;
    assert_eq!(removed.status.code(), Some(0));
    let gone = itda(&home, &["remove", "weather"], &[]).await;
    assert_eq!(gone.status.code(), Some(2));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn add_takes_a_ready_made_site_or_an_address() {
    let home = home();
    let bundled = itda(&home, &["add", "github"], &[]).await;
    assert_eq!(bundled.status.code(), Some(0));
    let review = String::from_utf8_lossy(&bundled.stderr);
    assert!(review.contains("https://api.github.com"), "{review}");
    let unknown = itda(&home, &["add", "gitlab"], &[]).await;
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("crates-io"));

    let router = Router::new().route(
        "/spec.yaml",
        get(|| async {
            "spec_version: 1
site: {name: remote, base_url: 'https://api.example.com'}
operations:
  - {name: ping, method: GET, path: /ping, effect: read, evidence: doc}
"
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let fetched = itda(&home, &["add", &format!("{base}/spec.yaml")], &[]).await;
    assert_eq!(fetched.status.code(), Some(0), "{fetched:?}");
    let missing = itda(&home, &["add", &format!("{base}/other.yaml")], &[]).await;
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        itda(&home, &["ls", "remote", "--json"], &[])
            .await
            .status
            .code(),
        Some(0)
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bare_itda_without_a_terminal_shows_the_command_list() {
    let home = home();
    let out = itda(&home, &[], &[]).await;
    assert!(out.status.success());
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("Usage: itda"), "{shown}");
    assert!(shown.contains("setup"), "{shown}");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_goes_from_a_page_to_working_actions_in_one_go() {
    let home = home();
    let router = Router::new().route(
        "/board",
        get(|| async {
            axum::response::Html(
                "<html lang='ko'><title>공지사항 - 사내</title><link rel='alternate' type='application/rss+xml' href='/missing.xml'><h1>공지</h1>
                 <form action='/board/search' method='get'><input name='q' required></form>
                 <table><thead><tr><th>No</th><th>Title</th></tr></thead>
                 <tr><td>2</td><td><a href='/n/2'>Second</a></td></tr>
                 <tr><td>1</td><td><a href='/n/1'>First</a></td></tr></table>
                 <form action='/board' method='post'><input name='title'></form></html>",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let url = format!("{base}/board");
    let done = itda(
        &home,
        &[
            "setup",
            &url,
            "--name",
            "board",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "ko")],
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(said.contains("목록 2건"), "{said}");
    assert!(said.contains("공지사항 목록 읽기"), "{said}");
    assert!(said.contains("뺐음"), "{said}");
    let spec = std::fs::read_to_string(home.join("specs/board/spec.yaml")).unwrap();
    assert!(spec.contains("name: get-board"));
    assert!(
        !spec.contains("name: feed"),
        "the broken feed was removed:\n{spec}"
    );
    assert!(spec.contains("name: post-board"));
    let called = itda(&home, &["call", "board.get-board", "--json"], &[]).await;
    assert_eq!(stdout_json(&called)["data"]["items"][0]["title"], "Second");

    let again = itda(
        &home,
        &[
            "setup",
            &url,
            "--name",
            "board",
            "--no-record",
            "--client",
            "none",
        ],
        &[],
    )
    .await;
    assert!(
        !again.status.success(),
        "without --yes an installed site is kept"
    );
    let no_terminal = itda(&home, &["setup", &url], &[]).await;
    assert_eq!(no_terminal.status.code(), Some(2));
    let one_line = itda(
        &home,
        &[
            &url,
            "--name",
            "board2",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[],
    )
    .await;
    assert!(one_line.status.success(), "{one_line:?}");
    assert!(home.join("specs/board2/spec.yaml").exists());
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_leaves_out_a_read_the_site_challenges_and_does_not_pause_it() {
    let home = home();
    let searched = Arc::new(AtomicUsize::new(0));
    let counted = searched.clone();
    let router = Router::new()
        .route(
            "/packages",
            get(|| async {
                axum::response::Html(
                    "<html><title>Packages</title><link rel='alternate' type='application/rss+xml' title='Updates' href='/rss.xml'><h1>Packages</h1>
                     <form action='/search/' method='get'><input name='q'></form>
                     <table><thead><tr><th>No</th><th>Name</th></tr></thead>
                     <tr><td>2</td><td><a href='/p/2'>Second</a></td></tr>
                     <tr><td>1</td><td><a href='/p/1'>First</a></td></tr></table></html>",
                )
            }),
        )
        .route(
            "/search/",
            get(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                async {
                    axum::response::Html(include_str!(
                        "../../../crates/net/tests/fixtures/client-challenge.html"
                    ))
                }
            }),
        )
        .route(
            "/rss.xml",
            get(|| async {
                (
                    [("content-type", "application/rss+xml")],
                    "<?xml version='1.0'?><rss version='2.0'><channel><title>Updates</title><item><title>One</title><link>/p/1</link></item></channel></rss>",
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let done = itda(
        &home,
        &[
            "setup",
            &format!("{base}/packages"),
            "--name",
            "pkgs",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(
        said.contains("/search/: the site does not give itda this page"),
        "{said}"
    );
    assert!(
        said.contains("not tried: the site refused an earlier read"),
        "{said}"
    );
    let spec = std::fs::read_to_string(home.join("specs/pkgs/spec.yaml")).unwrap();
    assert!(!spec.contains("path: /search/"), "{spec}");
    assert!(spec.contains("path: /packages"), "{spec}");
    assert!(spec.contains("path: /rss.xml"), "{spec}");
    assert_eq!(searched.load(Ordering::SeqCst), 1);

    // The site was not paused: the next check reads it, the feed too.
    let checked = itda(&home, &["check", "pkgs", "--json"], &[]).await;
    let report = stdout_json(&checked);
    assert_eq!(report["verdict"], "ok", "{report}");
    assert!(
        report["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["status"] == "ok"),
        "{report}"
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_on_a_known_host_uses_the_ready_made_definition() {
    let home = home();
    let added = itda(&home, &["add", "pypi"], &[]).await;
    assert!(added.status.success(), "{added:?}");
    // The page is never fetched: the ready-made definition is what would be
    // saved, and an installed one is kept without --yes, so this sends
    // nothing to pypi.org.
    let again = itda(
        &home,
        &[
            "setup",
            "https://pypi.org/project/requests/",
            "--no-record",
            "--client",
            "none",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&again.stderr);
    assert!(!again.status.success(), "{said}");
    assert!(said.contains("This is PyPI. Using its JSON API."), "{said}");
    assert!(
        !said.contains("The page opens without signing in."),
        "{said}"
    );
    assert!(said.contains("Kept the existing 'pypi'"), "{said}");
    std::fs::remove_dir_all(home).ok();
}

/// A GraphQL fixture: answers introspection with the prep crate's schema
/// when `enabled`, or refuses it the way servers do, and answers every
/// query with `{"data": {<field>: ...}}`.
async fn serve_graphql(enabled: bool) -> String {
    use axum::routing::post;
    let schema: Value = serde_json::from_str(include_str!(
        "../../../crates/prep/tests/fixtures/graphql_schema.json"
    ))
    .unwrap();
    let router = Router::new()
        .route(
            "/board",
            get(|| async {
                axum::response::Html(
                    "<title>Board</title><h1>Notices</h1>
                     <script src='/static/graphql.min.js'></script>
                     <script>fetch('/graphql', {method: 'POST'})</script>
                     <table><thead><tr><th>No</th><th>Title</th></tr></thead>
                     <tr><td>2</td><td><a href='/n/2'>Second</a></td></tr>
                     <tr><td>1</td><td><a href='/n/1'>First</a></td></tr></table>",
                )
            }),
        )
        .route(
            "/graphql",
            post(move |axum::Json(body): axum::Json<Value>| async move {
                let query = body["query"].as_str().unwrap_or_default().to_owned();
                if query.contains("__schema") {
                    return if enabled {
                        (axum::http::StatusCode::OK, axum::Json(schema.clone()))
                    } else {
                        (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::Json(json!({"errors": [{"message": "introspection is not allowed"}]})),
                        )
                    };
                }
                let field = query
                    .split_once("{ ")
                    .map(|(_, rest)| rest.split(['(', ' ']).next().unwrap_or_default())
                    .unwrap_or_default()
                    .to_owned();
                let answer = match field.as_str() {
                    "notices" => json!({"data": {"notices": [{"id": "1", "title": "first"}, {"id": "2", "title": "second"}]}}),
                    "version" => json!({"data": {"version": "1"}}),
                    _ => json!({"data": {field: null}}),
                };
                (axum::http::StatusCode::OK, axum::Json(answer))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_talks_graphql_gets_reads_from_its_schema() {
    let home = home();
    let base = serve_graphql(true).await;
    let printed = itda(
        &home,
        &[
            "draft",
            "page",
            &format!("{base}/board"),
            "--name",
            "gql",
            "--install",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&printed.stderr);
    assert!(printed.status.success(), "{said}");
    assert!(
        said.contains(&format!("Asked the GraphQL API at {base}/graphql")),
        "{said}"
    );
    assert!(said.contains("9 actions (9 reads, 0 changes)"), "{said}");
    let spec = std::fs::read_to_string(home.join("specs/gql/spec.yaml")).unwrap();
    assert!(spec.contains("name: notices"), "{spec}");
    assert!(spec.contains("name: version"), "{spec}");
    assert!(!spec.contains("createNotice"), "{spec}");

    let checked = itda(&home, &["check", "gql", "--json"], &[]).await;
    assert_eq!(checked.status.code(), Some(0), "{checked:?}");
    let report = stdout_json(&checked);
    let status = |action: &str| {
        report["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["action"] == action)
            .unwrap()
            .clone()
    };
    assert_eq!(status("notices")["status"], "ok");
    assert_eq!(status("version")["status"], "ok");
    assert_eq!(status("me")["status"], "ok");
    assert_eq!(status("notice")["status"], "skipped");
    assert_eq!(status("notice")["detail"], "needs id");
    assert_eq!(status("search")["detail"], "needs search_query");

    let called = itda(
        &home,
        &["call", "gql.notices", "--first", "1", "--json"],
        &[],
    )
    .await;
    assert!(called.status.success(), "{called:?}");
    assert_eq!(
        stdout_json(&called)["data"]["data"]["notices"][0]["title"],
        "first"
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_with_introspection_off_is_drafted_from_its_page_alone() {
    let home = home();
    let base = serve_graphql(false).await;
    let printed = itda(
        &home,
        &["draft", "page", &format!("{base}/board")],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&printed.stderr);
    assert!(printed.status.success(), "{said}");
    assert!(!said.contains("Asked the GraphQL API"), "{said}");
    assert!(said.contains("1 actions (1 reads, 0 changes)"), "{said}");
    let yaml = String::from_utf8_lossy(&printed.stdout);
    assert!(yaml.contains("name: get-board"), "{yaml}");
    assert!(!yaml.contains("/graphql"), "{yaml}");
    std::fs::remove_dir_all(home).ok();
}

/// A small OpenAPI document: one read and one change under `/items`.
fn description(server: Option<&str>) -> Value {
    let mut doc = json!({
        "openapi": "3.0.3",
        "info": {"title": "Items API", "version": "1.0"},
        "paths": {
            "/items": {
                "get": {"operationId": "listItems", "summary": "List the items",
                        "responses": {"200": {"content": {"application/json": {
                            "schema": {"type": "object", "required": ["items"]}}}}}},
                "post": {"operationId": "addItem",
                         "requestBody": {"content": {"application/json": {
                             "schema": {"type": "object", "properties": {"title": {"type": "string"}}}}}},
                         "responses": {"201": {"description": "added"}}}
            }
        }
    });
    if let Some(server) = server {
        doc["servers"] = json!([{"url": server}]);
    }
    doc
}

fn docs_page(link: bool) -> axum::response::Html<String> {
    let head = if link {
        "<link rel='service-desc' href='/openapi.json'>"
    } else {
        ""
    };
    axum::response::Html(format!(
        "<title>Docs - Items</title>{head}<h1>Items</h1>
         <table><thead><tr><th>No</th><th>Title</th></tr></thead>
         <tr><td>2</td><td><a href='/i/2'>Second</a></td></tr>
         <tr><td>1</td><td><a href='/i/1'>First</a></td></tr></table>"
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_links_its_api_description_gets_its_reads() {
    let home = home();
    let router = Router::new()
        .route("/docs", get(|| async { docs_page(true) }))
        .route(
            "/openapi.json",
            get(|| async { axum::Json(description(Some("/api"))) }),
        )
        .route(
            "/api/items",
            get(|| async { axum::Json(json!({"items": [{"title": "from the api"}]})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let installed = itda(
        &home,
        &[
            "draft",
            "page",
            &format!("{base}/docs"),
            "--name",
            "items",
            "--install",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&installed.stderr);
    assert!(installed.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "Read its API description at {base}/openapi.json: 1 reads, 1 changes noted."
        )),
        "{said}"
    );
    assert!(
        said.contains("note: POST /items: a change the API description lists"),
        "{said}"
    );
    assert!(said.contains("2 actions (2 reads, 0 changes)"), "{said}");
    let spec = std::fs::read_to_string(home.join("specs/items/spec.yaml")).unwrap();
    assert!(spec.contains("name: list-items"), "{spec}");
    assert!(spec.contains("path: /api/items"), "{spec}");
    assert!(
        spec.contains(&format!("and the API description at {base}/openapi.json")),
        "{spec}"
    );

    let called = itda(&home, &["call", "items.list-items", "--json"], &[]).await;
    assert!(called.status.success(), "{called:?}");
    assert_eq!(
        stdout_json(&called)["data"]["items"][0]["title"],
        "from the api"
    );
    let checked = itda(&home, &["check", "items", "--json"], &[]).await;
    assert!(checked.status.success(), "{checked:?}");
    assert_eq!(stdout_json(&checked)["verdict"], "ok");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_well_known_path_answers_with_the_api_description() {
    let home = home();
    let others = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route("/board", get(|| async { docs_page(false) }))
        .route(
            "/openapi.json",
            get(|| async { axum::Json(description(None)) }),
        )
        .route(
            "/items",
            get(|| async { axum::Json(json!({"items": [{"title": "probed"}]})) }),
        )
        .fallback({
            let others = others.clone();
            move || async move {
                others.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::NOT_FOUND
            }
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let done = itda(
        &home,
        &[
            "setup",
            &format!("{base}/board"),
            "--name",
            "board",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "ko")],
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "{base}/openapi.json의 API 설명을 읽었습니다: 읽기 1개, 메모로 남긴 변경 1개."
        )),
        "{said}"
    );
    assert!(said.contains("list-items"), "{said}");
    let spec = std::fs::read_to_string(home.join("specs/board/spec.yaml")).unwrap();
    assert!(spec.contains("path: /items"), "{spec}");
    assert!(
        !spec.contains("add-item"),
        "changes are noted, not drafted: {spec}"
    );
    let called = itda(&home, &["call", "board.list-items", "--json"], &[]).await;
    assert_eq!(stdout_json(&called)["data"]["items"][0]["title"], "probed");
    assert_eq!(
        others.load(Ordering::SeqCst),
        0,
        "only /openapi.json is probed"
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pages_link_header_names_the_api_description() {
    let home = home();
    let others = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route(
            "/board",
            get(|| async {
                (
                    [("link", r#"</spec/items.json>; rel="service-desc""#)],
                    docs_page(false),
                )
            }),
        )
        .route(
            "/spec/items.json",
            get(|| async { axum::Json(description(None)) }),
        )
        .route(
            "/items",
            get(|| async { axum::Json(json!({"items": [{"title": "linked"}]})) }),
        )
        .fallback({
            let others = others.clone();
            move || async move {
                others.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::NOT_FOUND
            }
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let english = [("ITDA_LANG", "en")];
    let drafted = itda(
        &home,
        &["draft", "page", &format!("{base}/board")],
        &english,
    )
    .await;
    let said = String::from_utf8_lossy(&drafted.stderr);
    assert!(drafted.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "Read its API description at {base}/spec/items.json"
        )),
        "{said}"
    );
    let done = itda(
        &home,
        &[
            "setup",
            &format!("{base}/board"),
            "--name",
            "board",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &english,
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "Read its API description at {base}/spec/items.json"
        )),
        "{said}"
    );
    let called = itda(&home, &["call", "board.list-items", "--json"], &[]).await;
    assert_eq!(stdout_json(&called)["data"]["items"][0]["title"], "linked");
    assert_eq!(
        others.load(Ordering::SeqCst),
        0,
        "/openapi.json is not probed when the header names the description"
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_description_leaves_the_page_draft_as_it_is() {
    let home = home();
    let router = Router::new()
        .route("/docs", get(|| async { docs_page(true) }))
        .route("/openapi.json", get(|| async { axum::Json(json!({})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let printed = itda(
        &home,
        &["draft", "page", &format!("{base}/docs")],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&printed.stderr);
    assert!(printed.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "note: Could not read {base}/openapi.json as an API description"
        )),
        "{said}"
    );
    assert!(!said.contains("Read its API description"), "{said}");
    assert!(said.contains("1 actions (1 reads, 0 changes)"), "{said}");
    assert!(String::from_utf8_lossy(&printed.stdout).contains("name: list-docs"));
    std::fs::remove_dir_all(home).ok();
}

async fn serve_html(path: &'static str, body: &'static str) -> String {
    let router = Router::new().route(path, get(move || async move { axum::response::Html(body) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blocked_page_is_named_by_what_the_gate_saw() {
    use axum::http::StatusCode;
    let router = Router::new()
        .route(
            "/cf",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    [("server", "cloudflare"), ("content-type", "text/html")],
                    "<title>Just a moment...</title>",
                )
            }),
        )
        .route(
            "/slow",
            get(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "120"), ("content-type", "text/html")],
                    "<p>slow down</p>",
                )
            }),
        )
        .route(
            "/r/rust",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [
                        ("location", "/login/?dest=%2Fr%2Frust"),
                        ("retry-after", "0"),
                    ],
                    "",
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    // Each run gets its own data directory: a block pauses the site in it.
    let dir = home();
    let english = [("ITDA_LANG", "en")];
    let blocked = itda(&dir, &["draft", "page", &format!("{base}/cf")], &english).await;
    assert_eq!(blocked.status.code(), Some(12), "{blocked:?}");
    let err = String::from_utf8_lossy(&blocked.stderr);
    assert!(
        err.contains("error[blocked_by_site]: 127.0.0.1 is behind Cloudflare and refused itda."),
        "{err}"
    );
    assert!(!err.contains("HTTP 403"), "{err}");
    std::fs::remove_dir_all(dir).ok();

    let dir = home();
    let korean = itda(
        &dir,
        &["draft", "page", &format!("{base}/cf")],
        &[("ITDA_LANG", "ko")],
    )
    .await;
    assert_eq!(korean.status.code(), Some(12));
    assert!(
        String::from_utf8_lossy(&korean.stderr)
            .contains("Cloudflare 뒤에 있어 itda를 거부했습니다"),
        "{korean:?}"
    );
    std::fs::remove_dir_all(dir).ok();

    let dir = home();
    let limited = itda(&dir, &["draft", "page", &format!("{base}/slow")], &english).await;
    assert_eq!(limited.status.code(), Some(13), "{limited:?}");
    let err = String::from_utf8_lossy(&limited.stderr);
    assert!(
        err.contains("error[site_rate_limited]: 127.0.0.1 limits how often it is read and asked itda to wait 120 s."),
        "{err}"
    );
    std::fs::remove_dir_all(dir).ok();

    let dir = home();
    let signin = itda(
        &dir,
        &["draft", "page", &format!("{base}/r/rust")],
        &english,
    )
    .await;
    assert_eq!(signin.status.code(), Some(10), "{signin:?}");
    let err = String::from_utf8_lossy(&signin.stderr);
    assert!(
        err.contains("error[auth_required]: 127.0.0.1 sends itda to a sign-in page instead of showing /r/rust."),
        "{err}"
    );
    assert!(err.contains("itda record"), "{err}");
    std::fs::remove_dir_all(dir).ok();

    let dir = home();
    let setup = itda(
        &dir,
        &[
            "setup",
            &format!("{base}/cf"),
            "--no-record",
            "--client",
            "cursor",
            "--yes",
        ],
        &english,
    )
    .await;
    assert!(!setup.status.success());
    let err = String::from_utf8_lossy(&setup.stderr);
    assert!(
        err.contains("127.0.0.1 is behind Cloudflare and refused itda."),
        "{err}"
    );
    assert!(!err.contains("needs a sign-in"), "{err}");
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn check_counts_the_records_of_a_typed_embedded_read() {
    let home = home();
    let base = serve_html(
        "/products",
        r#"<!doctype html><html><head><title>Shop</title></head><body><div id="__next"></div>
<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"products":[{"id":1,"name":"Lamp","token":"private-value"},{"id":2,"name":"Desk","token":"private-value"},{"id":3,"name":"Chair","token":"private-value"}]}},"page":"/products","buildId":"abc"}</script>
</body></html>"#,
    )
    .await;
    let installed = itda(
        &home,
        &[
            "draft",
            "page",
            &format!("{base}/products"),
            "--name",
            "shop",
            "--install",
        ],
        &[],
    )
    .await;
    assert!(installed.status.success(), "{installed:?}");

    let checked = itda(&home, &["check", "shop", "--json"], &[]).await;
    assert!(checked.status.success(), "{checked:?}");
    let report = stdout_json(&checked);
    assert_eq!(report["results"][0]["action"], "list-products");
    assert_eq!(report["results"][0]["status"], "ok");
    assert_eq!(report["results"][0]["records"], 3, "{report}");

    let called = itda(&home, &["call", "shop.list-products", "--json"], &[]).await;
    assert!(called.status.success(), "{called:?}");
    let data = &stdout_json(&called)["data"];
    assert_eq!(data.as_array().map(Vec::len), Some(3));
    assert_eq!(data[0]["name"], "Lamp");
    assert_eq!(data[0]["id"], 1);
    assert!(data[0].get("token").is_none(), "{data}");
    std::fs::remove_dir_all(home).ok();
}

const BOARD_V1: &str = "<html><head><title>Notices</title><link rel='alternate' type='application/rss+xml' href='/feed.xml'><script type='application/ld+json'>[{\"@type\":\"NewsArticle\",\"headline\":\"Third\",\"url\":\"/n/3\"},{\"@type\":\"NewsArticle\",\"headline\":\"Second\",\"url\":\"/n/2\"},{\"@type\":\"NewsArticle\",\"headline\":\"First\",\"url\":\"/n/1\"}]</script></head><body><h1>Notices</h1>
<table><thead><tr><th>No</th><th>Title</th></tr></thead>
<tr><td>2</td><td><a href='/n/2'>Second</a></td></tr>
<tr><td>1</td><td><a href='/n/1'>First</a></td></tr></table></body></html>";

/// The same board after a redesign: the heading and a column are renamed
/// and the feed moved.
const BOARD_V2: &str = "<html><head><title>Notices</title><link rel='alternate' type='application/rss+xml' href='/rss.xml'><script type='application/ld+json'>[{\"@type\":\"NewsArticle\",\"headline\":\"Third\",\"url\":\"/n/3\"},{\"@type\":\"NewsArticle\",\"headline\":\"Second\",\"url\":\"/n/2\"},{\"@type\":\"NewsArticle\",\"headline\":\"First\",\"url\":\"/n/1\"}]</script></head><body><h2>Notices</h2>
<table><thead><tr><th>Subject</th><th>Posted</th></tr></thead>
<tr><td><a href='/n/2'>Second</a></td><td>today</td></tr>
<tr><td><a href='/n/1'>First</a></td><td>yesterday</td></tr></table></body></html>";

const FEED: &str = "<?xml version='1.0'?><rss><channel><title>Notices</title></channel></rss>";

/// A board whose page changes once `redesigned` is set.
async fn serve_changing_board() -> (String, Arc<std::sync::atomic::AtomicBool>) {
    use axum::{http::StatusCode, response::IntoResponse};
    let redesigned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = redesigned.clone();
    let board = move || {
        let flag = flag.clone();
        async move {
            axum::response::Html(if flag.load(Ordering::SeqCst) {
                BOARD_V2
            } else {
                BOARD_V1
            })
        }
    };
    let feed = |expect: bool| {
        let flag = redesigned.clone();
        move || {
            let flag = flag.clone();
            async move {
                if flag.load(Ordering::SeqCst) == expect {
                    ([("content-type", "application/rss+xml")], FEED).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }
        }
    };
    let router = Router::new()
        .route("/board", get(board))
        .route("/feed.xml", get(feed(false)))
        .route("/rss.xml", get(feed(true)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, redesigned)
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_drafts_a_changed_site_again_and_keeps_what_was_edited_by_hand() {
    let home = home();
    let (base, redesigned) = serve_changing_board().await;
    let url = format!("{base}/board");
    let done = itda(
        &home,
        &[
            "setup",
            &url,
            "--name",
            "board",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    assert!(done.status.success(), "{done:?}");
    let sources: Value =
        serde_json::from_slice(&std::fs::read(home.join("specs/board/sources.json")).unwrap())
            .unwrap();
    assert_eq!(sources["pages"], json!([url]));
    assert!(sources["drafted"]["get-board"].is_string(), "{sources}");
    assert!(sources["drafted"]["feed"].is_string(), "{sources}");

    // The person renames the embedded list's read by hand.
    let path = home.join("specs/board/spec.yaml");
    let spec = std::fs::read_to_string(&path).unwrap();
    assert!(
        spec.contains("name: list-news-articles\n  desc: "),
        "{spec}\n{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let edited = spec.replace(
        "name: list-news-articles\n  desc: ",
        "name: list-news-articles\n  desc: Find the latest notices, not ",
    );
    assert_ne!(edited, spec);
    std::fs::write(&path, &edited).unwrap();

    // Nothing changed yet: a refresh has nothing to write.
    let same = itda(
        &home,
        &["refresh", "board", "--yes"],
        &[("ITDA_LANG", "en")],
    )
    .await;
    assert!(same.status.success(), "{same:?}");
    let said = String::from_utf8_lossy(&same.stderr);
    assert!(said.contains("draft the same actions"), "{said}");
    assert!(said.contains("= list-news-articles"), "{said}");

    redesigned.store(true, Ordering::SeqCst);
    let changed = itda(&home, &["check", "board"], &[("ITDA_LANG", "en")]).await;
    assert_eq!(changed.status.code(), Some(11), "{changed:?}");
    let text = String::from_utf8_lossy(&changed.stdout);
    assert!(
        text.contains("verdict  site_changed: the site changed; run itda refresh board"),
        "{text}"
    );
    let all = itda(&home, &["check", "--all"], &[("ITDA_LANG", "en")]).await;
    assert_eq!(all.status.code(), Some(11), "{all:?}");
    let text = String::from_utf8_lossy(&all.stdout);
    assert!(text.starts_with("board  site_changed: "), "{text}");
    assert_eq!(text.lines().count(), 1, "{text}");

    let asked = itda(&home, &["refresh", "board"], &[("ITDA_LANG", "en")]).await;
    assert!(asked.status.success(), "{asked:?}");
    let said = String::from_utf8_lossy(&asked.stderr);
    assert!(said.contains("As installed: site_changed"), "{said}");
    assert!(said.contains("~ get-board"), "{said}");
    assert!(said.contains("changed: "), "{said}");
    assert!(said.contains("~ feed"), "{said}");
    assert!(said.contains("= list-news-articles"), "{said}");
    assert!(said.contains("Kept 'board' as it is"), "{said}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edited,
        "without a yes nothing is written"
    );

    let written = itda(&home, &["refresh", "board", "-y"], &[("ITDA_LANG", "en")]).await;
    assert!(written.status.success(), "{written:?}");
    let said = String::from_utf8_lossy(&written.stderr);
    assert!(said.contains("Updated 'board'"), "{said}");
    let text = String::from_utf8_lossy(&written.stdout);
    assert!(
        text.contains("verdict  ok: every read still matches"),
        "{text}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("desc: Find the latest notices, not "),
        "{after}"
    );
    assert!(after.contains("subject:"), "{after}");
    assert!(after.contains("path: /rss.xml"), "{after}");
    assert!(!after.contains("/feed.xml"), "{after}");
    let sources: Value =
        serde_json::from_slice(&std::fs::read(home.join("specs/board/sources.json")).unwrap())
            .unwrap();
    assert_eq!(sources["pages"], json!([url]));

    let ok = itda(&home, &["check", "board", "--json"], &[("ITDA_LANG", "en")]).await;
    assert_eq!(ok.status.code(), Some(0), "{ok:?}");
    assert_eq!(stdout_json(&ok)["verdict"], "ok");
    let called = itda(
        &home,
        &["call", "board.get-board", "--json"],
        &[("ITDA_LANG", "en")],
    )
    .await;
    assert_eq!(
        stdout_json(&called)["data"]["items"][0]["subject"],
        "Second"
    );

    // A site installed by hand has no sources to refresh from.
    install(&home, &base);
    let none = itda(&home, &["refresh", "fixture", "-y"], &[("ITDA_LANG", "en")]).await;
    assert_eq!(none.status.code(), Some(2), "{none:?}");
    let said = String::from_utf8_lossy(&none.stderr);
    assert!(said.contains("No known sources"), "{said}");
    std::fs::remove_dir_all(home).ok();
}

/// A WordPress look-alike: the page advertises its API root, the root
/// answers the way WordPress does, and two reads of the template answer.
async fn serve_wordpress(api_answers: bool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let page = format!(
        "<html><head><title>My Blog</title>
         <link rel='https://api.w.org/' href='{base}/wp-json/' />
         <meta name='generator' content='WordPress 6.6' /></head>
         <body class='home blog'><h1>My Blog</h1>
         <table><thead><tr><th>No</th><th>Title</th></tr></thead>
         <tr><td>2</td><td><a href='/p/2'>Second</a></td></tr>
         <tr><td>1</td><td><a href='/p/1'>First</a></td></tr></table></body></html>"
    );
    let mut router =
        Router::new().route("/", get(move || async move { axum::response::Html(page) }));
    if api_answers {
        router = router
            .route(
                "/wp-json/",
                get(|| async {
                    axum::Json(json!({
                        "name": "My Blog", "description": "notes", "url": "http://blog.test",
                        "namespaces": ["oembed/1.0", "wp/v2"], "routes": {}
                    }))
                }),
            )
            .route(
                "/wp-json/wp/v2/posts",
                get(|| async {
                    axum::Json(json!([{
                        "id": 7, "date": "2026-10-01T10:00:00", "modified": "2026-10-02T10:00:00",
                        "slug": "hello", "status": "publish", "link": "http://blog.test/hello",
                        "title": {"rendered": "Hello"}, "excerpt": {"rendered": "<p>Hi</p>"},
                        "author": 1, "categories": [1], "tags": []
                    }]))
                }),
            )
            .route(
                "/wp-json/wp/v2/categories",
                get(|| async {
                    axum::Json(json!([{"id": 1, "count": 1, "name": "News", "slug": "news", "parent": 0, "description": "", "link": "http://blog.test/c/news"}]))
                }),
            );
    }
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_running_known_software_gets_its_template() {
    let home = home();
    let base = serve_wordpress(true).await;
    let done = itda(
        &home,
        &[
            "setup",
            &format!("{base}/"),
            "--name",
            "blog",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "This looks like WordPress. Using its API at {base}/wp-json."
        )),
        "{said}"
    );
    let spec = std::fs::read_to_string(home.join("specs/blog/spec.yaml")).unwrap();
    assert!(
        spec.starts_with(
            "# Drafted by `itda setup` from the WordPress template and a page of 127.0.0.1."
        ),
        "{spec}"
    );
    assert!(
        spec.contains(&format!("base_url: {base}/wp-json")),
        "{spec}"
    );
    assert!(spec.contains("name: posts"), "{spec}");
    assert!(spec.contains("name: categories"), "{spec}");
    assert!(spec.contains("name: create-post"), "{spec}");
    assert!(spec.contains("store:blog/api-key"), "{spec}");
    assert!(
        !spec.contains("name: tags") && !spec.contains("name: comments"),
        "reads the fixture does not answer are removed: {spec}"
    );
    // The page's own table joins the template under the page's address.
    assert!(spec.contains("html"), "{spec}");
    assert!(said.contains("2 records"), "{said}");
    let posts = itda(&home, &["call", "blog.posts", "--json"], &[]).await;
    assert_eq!(stdout_json(&posts)["data"][0]["title"], "Hello");
    let checked = itda(&home, &["check", "blog", "--json"], &[]).await;
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert_eq!(stdout_json(&checked)["verdict"], "ok");
    // The sources name the template, and a refresh fills it again.
    let sources: Value =
        serde_json::from_slice(&std::fs::read(home.join("specs/blog/sources.json")).unwrap())
            .unwrap();
    assert_eq!(sources["template"], "wordpress", "{sources}");
    assert_eq!(sources["base_url"], format!("{base}/wp-json"), "{sources}");
    let refreshed = itda(&home, &["refresh", "blog", "-y"], &[("ITDA_LANG", "en")]).await;
    let said = String::from_utf8_lossy(&refreshed.stderr);
    assert!(refreshed.status.success(), "{said}");
    assert!(said.contains("its template"), "{said}");
    assert!(
        said.contains(&format!(
            "This looks like WordPress. Using its API at {base}/wp-json."
        )),
        "{said}"
    );
    let spec = std::fs::read_to_string(home.join("specs/blog/spec.yaml")).unwrap();
    assert!(
        spec.contains("name: posts") && spec.contains("name: create-post"),
        "{spec}"
    );
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fingerprint_the_api_does_not_confirm_falls_back_to_the_page() {
    let home = home();
    let base = serve_wordpress(false).await;
    let done = itda(
        &home,
        &[
            "setup",
            &format!("{base}/"),
            "--name",
            "blog",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(!said.contains("This looks like"), "{said}");
    let spec = std::fs::read_to_string(home.join("specs/blog/spec.yaml")).unwrap();
    assert!(!spec.contains("wp/v2"), "{spec}");
    assert!(spec.contains("html"), "{spec}");

    // --no-template reads the page like any other even when the API answers.
    let base = serve_wordpress(true).await;
    let plain = itda(
        &home,
        &[
            "setup",
            &format!("{base}/"),
            "--name",
            "plain",
            "--no-record",
            "--no-template",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "en")],
    )
    .await;
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    let spec = std::fs::read_to_string(home.join("specs/plain/spec.yaml")).unwrap();
    assert!(!spec.contains("wp/v2"), "{spec}");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_template_can_be_forced_by_name() {
    let home = home();
    let router = Router::new()
        .route(
            "/explore/repos",
            get(|| async { axum::response::Html("<html><head><title>Repos</title></head><body><h1>Repositories</h1></body></html>") }),
        )
        .route(
            "/api/v1/version",
            get(|| async { axum::Json(json!({"version": "1.0.0-dev+gitea-1.22.0"})) }),
        )
        .route(
            "/api/v1/repos/search",
            get(|| async {
                axum::Json(json!({"ok": true, "data": [{"id": 1, "full_name": "org/repo", "description": "", "default_branch": "main", "private": false, "fork": false, "stars_count": 3, "forks_count": 0, "open_issues_count": 1, "updated_at": "2026-10-01T00:00:00Z", "html_url": "http://git.test/org/repo"}]}))
            }),
        )
        .route(
            "/api/v1/orgs",
            get(|| async { axum::Json(json!([{"id": 1, "name": "org", "full_name": "The Org", "description": "", "website": "", "visibility": "public"}])) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let done = itda(
        &home,
        &[
            "setup",
            &format!("{base}/explore/repos"),
            "--as",
            "gitea",
            "--name",
            "forge",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[("ITDA_LANG", "ko")],
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(
        said.contains(&format!(
            "Forgejo로 보입니다. {base}/api/v1의 API를 씁니다."
        )),
        "{said}"
    );
    let spec = std::fs::read_to_string(home.join("specs/forge/spec.yaml")).unwrap();
    assert!(spec.contains("name: search-repos"), "{spec}");
    assert!(spec.contains("name: orgs"), "{spec}");
    assert!(spec.contains("name: create-issue"), "{spec}");
    let repos = itda(&home, &["call", "forge.search-repos", "--json"], &[]).await;
    assert_eq!(stdout_json(&repos)["data"][0]["full_name"], "org/repo");

    let unknown = itda(
        &home,
        &[
            "setup",
            &format!("{base}/explore/repos"),
            "--as",
            "phabricator",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &[],
    )
    .await;
    assert_eq!(unknown.status.code(), Some(2), "{unknown:?}");
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("gitlab, jira, confluence"),
        "{}",
        String::from_utf8_lossy(&unknown.stderr)
    );
    std::fs::remove_dir_all(home).ok();
}

/// A page drawn by scripts, served three ways: one that fetches its list on
/// load and more on the first scroll, one that fetches nothing, and one whose
/// script sends the browser to a sign-in page.
#[cfg(unix)]
async fn serve_spa() -> String {
    use axum::response::Html;
    let shell = |script: &'static str| {
        move || async move {
            Html(format!(
                "<!doctype html><html><head><title>Items</title></head><body>
                 <div id='app' style='min-height: 4000px'>loading</div>
                 <script src='{script}'></script></body></html>"
            ))
        }
    };
    let js = |text: &'static str| {
        move || async move { ([("content-type", "application/javascript")], text) }
    };
    let router = Router::new()
        .route("/app", get(shell("/app.js")))
        .route(
            "/app.js",
            get(js("fetch('/api/items').then(r => r.json()).then(d => { document.getElementById('app').textContent = d.items.length + ' items'; });
                   let more = false;
                   addEventListener('scroll', () => {
                     if (!more && innerHeight + scrollY >= document.body.scrollHeight - 10) { more = true; fetch('/api/more'); }
                   });")),
        )
        .route(
            "/api/items",
            get(|| async { axum::Json(json!({"items": [{"id": 1, "title": "first"}]})) }),
        )
        .route(
            "/api/more",
            get(|| async { axum::Json(json!({"items": [{"id": 2, "title": "second"}]})) }),
        )
        .route(
            "/plain",
            get(|| async {
                Html("<title>Plain</title><p>A page with a title, a paragraph and no script at all, which shows a plain fetch everything it has.</p>")
            }),
        )
        .route("/empty", get(shell("/empty.js")))
        .route(
            "/empty.js",
            get(js("document.getElementById('app').textContent = 'nothing here';")),
        )
        .route("/private", get(shell("/private.js")))
        .route(
            "/private.js",
            get(js("location.replace('/login?next=%2Fprivate');")),
        )
        .route(
            "/login",
            get(|| async { Html("<title>Sign in</title><form><input type='password' name='p'></form>") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

/// The browser `ITDA_TEST_BROWSER` names, wrapped to run headless; `None`
/// when the variable is not set.
#[cfg(unix)]
fn headless_browser(dir: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
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

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn adaptive_calls_recover_and_remember_browser_transport() {
    use axum::{http::StatusCode, response::IntoResponse};
    let home = home();
    let Some(browser) = headless_browser(&home) else {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    };
    let direct = Arc::new(AtomicUsize::new(0));
    let fetched = Arc::new(AtomicUsize::new(0));
    let browser_identity = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let router = Router::new().route(
        "/items",
        get({
            let direct = direct.clone();
            let fetched = fetched.clone();
            let browser_identity = browser_identity.clone();
            move |headers: HeaderMap| {
                let direct = direct.clone();
                let fetched = fetched.clone();
                let browser_identity = browser_identity.clone();
                async move {
                    if headers.contains_key("sec-fetch-mode") {
                        fetched.fetch_add(1, Ordering::SeqCst);
                        axum::Json(json!({"items":[{"title":"recovered"}]})).into_response()
                    } else {
                        direct.fetch_add(1, Ordering::SeqCst);
                        browser_identity.store(
                            headers
                                .get("user-agent")
                                .and_then(|v| v.to_str().ok())
                                .is_some_and(|v| v.contains("Chrome/")),
                            Ordering::SeqCst,
                        );
                        (
                            StatusCode::FORBIDDEN,
                            axum::response::Html("<title>Just a moment</title>"),
                        )
                            .into_response()
                    }
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    install(&home, &format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    for _ in 0..2 {
        let result = itda(
            &home,
            &["call", "fixture.list-items", "--json"],
            &[
                ("ITDA_TRANSPORT", "auto"),
                ("ITDA_BROWSER", browser.to_str().unwrap()),
            ],
        )
        .await;
        assert!(result.status.success(), "{result:?}");
        assert_eq!(
            stdout_json(&result)["data"]["items"][0]["title"],
            "recovered"
        );
    }
    assert_eq!(direct.load(Ordering::SeqCst), 1);
    assert_eq!(fetched.load(Ordering::SeqCst), 2);
    assert!(browser_identity.load(Ordering::SeqCst));
    let stopped = itda(
        &home,
        &["__browser-stop"],
        &[("ITDA_BROWSER", browser.to_str().unwrap())],
    )
    .await;
    assert!(stopped.status.success(), "{stopped:?}");
    server.abort();
    std::fs::remove_dir_all(home).unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn setup_watches_what_a_scripted_page_loads_before_asking_to_record() {
    let home = home();
    let Some(browser) = headless_browser(&home) else {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    };
    let browser = browser.to_str().unwrap().to_owned();
    let env = [("ITDA_BROWSER", browser.as_str()), ("ITDA_LANG", "en")];
    let base = serve_spa().await;

    // The calls the page makes on load and on the first scroll become reads.
    let done = itda(
        &home,
        &[
            "setup",
            &format!("{base}/app"),
            "--name",
            "app",
            "--watch-only",
            "--client",
            "none",
            "--yes",
        ],
        &env,
    )
    .await;
    let said = String::from_utf8_lossy(&done.stderr);
    assert!(done.status.success(), "{said}");
    assert!(said.contains("Opening your browser on 127.0.0.1"), "{said}");
    assert!(said.contains("Watched the page load: "), "{said}");
    assert!(said.contains(" calls, 2 reads drafted."), "{said}");
    assert!(!said.contains("Open the browser and record?"), "{said}");
    let spec = std::fs::read_to_string(home.join("specs/app/spec.yaml")).unwrap();
    assert!(spec.contains("path: /api/items"), "{spec}");
    assert!(
        spec.contains("path: /api/more"),
        "the scroll was watched too:\n{spec}"
    );
    let called = itda(&home, &["call", "app.list-items", "--json"], &[]).await;
    assert!(called.status.success(), "{called:?}");
    assert_eq!(stdout_json(&called)["data"]["items"][0]["title"], "first");

    // A page that loads nothing falls through to the question, which
    // --watch-only answers by stopping.
    let nothing = itda(
        &home,
        &[
            "setup",
            &format!("{base}/empty"),
            "--name",
            "empty",
            "--watch-only",
            "--client",
            "none",
            "--yes",
        ],
        &env,
    )
    .await;
    let said = String::from_utf8_lossy(&nothing.stderr);
    assert_eq!(nothing.status.code(), Some(2), "{said}");
    assert!(said.contains(" calls, nothing to draft."), "{said}");
    assert!(said.contains("(it is drawn by scripts)"), "{said}");
    assert!(said.contains("--watch-only skips recording"), "{said}");
    assert!(!home.join("specs/empty").exists());

    // A page whose scripts go to a sign-in page stops with the refusal;
    // nothing is typed there.
    let private = itda(
        &home,
        &[
            "setup",
            &format!("{base}/private"),
            "--name",
            "private",
            "--watch-only",
            "--client",
            "none",
            "--yes",
        ],
        &env,
    )
    .await;
    let said = String::from_utf8_lossy(&private.stderr);
    assert!(!private.status.success(), "{said}");
    assert!(
        said.contains("127.0.0.1 sends itda to a sign-in page instead of showing /private."),
        "{said}"
    );
    assert!(said.contains("(it needs a sign-in)"), "{said}");
    assert!(!home.join("specs/private").exists());

    // A bare page without scripts cannot load anything by itself: no window.
    let plain = itda(
        &home,
        &[
            "setup",
            &format!("{base}/plain"),
            "--name",
            "plain",
            "--watch-only",
            "--client",
            "none",
            "--yes",
        ],
        &env,
    )
    .await;
    let said = String::from_utf8_lossy(&plain.stderr);
    assert!(plain.status.success(), "{said}");
    assert!(said.contains("Found: only a title."), "{said}");
    assert!(!said.contains("Opening your browser"), "{said}");
    assert!(home.join("specs/plain/spec.yaml").exists());

    // --no-record never opens the browser.
    let no_browser = itda(
        &home,
        &[
            "setup",
            &format!("{base}/app"),
            "--name",
            "app2",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &env,
    )
    .await;
    let said = String::from_utf8_lossy(&no_browser.stderr);
    assert!(!no_browser.status.success(), "{said}");
    assert!(!said.contains("Opening your browser"), "{said}");
    assert!(said.contains("--no-record forbids the browser"), "{said}");
    std::fs::remove_dir_all(home).ok();
}

/// A site behind a sign-in form, the way older intranets are built: the
/// list redirects anonymous readers to the form (with a parameter itda
/// refuses in an address), the form hands out a token, a change answers with
/// a redirect, and the page the change's token comes from sets a cookie the
/// change must carry back.
async fn serve_intranet(posts: Arc<AtomicUsize>) -> String {
    use axum::{
        http::StatusCode,
        response::{Html, IntoResponse, Redirect},
    };
    fn signed_in(headers: &HeaderMap) -> bool {
        headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.contains("SID=lab-session"))
    }
    let router = Router::new()
        .route(
            "/notices",
            get(|headers: HeaderMap| async move {
                if !signed_in(&headers) {
                    return Redirect::to("/login?next=%2Fnotices&reauth=1").into_response();
                }
                Html(
                    "<h1>Notices</h1><table><thead><tr><th>No</th><th>Title</th></tr></thead>
                     <tr class='notice'><td>1</td><td><a href='/n/1'>First</a></td></tr>
                     <tr class='notice'><td>2</td><td><a href='/n/2'>Second</a></td></tr></table>",
                )
                .into_response()
            })
            .post(
                move |headers: HeaderMap,
                      axum::Form(form): axum::Form<std::collections::HashMap<String, String>>| {
                    let posts = posts.clone();
                    async move {
                        posts.fetch_add(1, Ordering::SeqCst);
                        if !signed_in(&headers) {
                            return Redirect::to("/login?next=%2Fnotices").into_response();
                        }
                        let cookie = headers
                            .get("cookie")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default();
                        if !cookie.contains("FORM=visit-7")
                            || form.get("_csrf").map(String::as_str) != Some("tok-7")
                        {
                            return StatusCode::FORBIDDEN.into_response();
                        }
                        Redirect::to("/n/3").into_response()
                    }
                },
            ),
        )
        .route(
            "/notices/new",
            get(|headers: HeaderMap| async move {
                if !signed_in(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                (
                    [("set-cookie", "FORM=visit-7; Path=/")],
                    Html("<form method='post' action='/notices'><input type='hidden' name='_csrf' value='tok-7'><input name='title'></form>"),
                )
                    .into_response()
            }),
        )
        .route(
            "/login",
            get(|| async {
                Html("<title>Sign in</title><form method='post' action='/login'><input type='hidden' name='_csrf' value='l1'><input name='username'><input type='password' name='password'></form>")
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn install_intranet(home: &Path, base: &str) {
    let dir = home.join("specs/intranet");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("spec.yaml"),
        format!(
            "spec_version: 1
site: {{name: intranet, base_url: {base}}}
auth:
  - name: session
    kind: browser
    value_from: cookie:SID
    login: {{path: /login, user: username, password: password}}
operations:
  - name: list-notices
    method: GET
    path: /notices
    auth: session
    effect: read
    evidence: doc
    response:
      format: html
      html:
        items: tr.notice
        fields:
          title: {{selector: 'td:nth-child(2) a', required: true}}
  - name: post-notice
    method: POST
    path: /notices
    auth: session
    effect: write
    evidence: doc
    preflight:
      - name: form
        path: /notices/new
        extract: {{csrf: 'css:input[name=_csrf]@value'}}
    body:
      form:
        title: {{type: string, required: true}}
        _csrf: {{type: string, default: '{{{{step.form.csrf}}}}'}}
"
        ),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_carries_the_cookie_its_form_page_set_and_a_redirect_means_it_landed() {
    let home = home();
    let posts = Arc::new(AtomicUsize::new(0));
    let base = serve_intranet(posts.clone()).await;
    install_intranet(&home, &base);
    let session = [("ITDA_SECRET__INTRANET__DEFAULT__SESSION", "SID=lab-session")];

    let read = itda(
        &home,
        &["call", "intranet.list-notices", "--json"],
        &session,
    )
    .await;
    assert!(read.status.success(), "{read:?}");
    assert_eq!(stdout_json(&read)["data"]["items"][1]["title"], "Second");

    let waiting = itda(
        &home,
        &["call", "intranet.post-notice", "--title", "hi", "--json"],
        &session,
    )
    .await;
    let id = stdout_json(&waiting)["approval"]
        .as_str()
        .unwrap()
        .to_owned();
    // Approved the way `itda approve` does it, without a terminal to type in.
    let store =
        itda_policy::Approvals::open(itda_store::Store::open(&home.join("itda.db")).unwrap())
            .unwrap();
    store.claim(&id).await.unwrap();
    let landed = itda(
        &home,
        &[
            "call",
            "intranet.post-notice",
            "--title",
            "hi",
            "--approval",
            &id,
            "--json",
        ],
        &session,
    )
    .await;
    assert!(landed.status.success(), "{landed:?}");
    let envelope = stdout_json(&landed);
    assert_eq!(envelope["meta"]["http_status"], 303);
    assert_eq!(envelope["data"]["location"], format!("{base}/n/3"));
    assert_eq!(posts.load(Ordering::SeqCst), 1);

    // With a session the site no longer accepts, the form page itself
    // refuses, so the change stops before it is sent.
    let expired = [("ITDA_SECRET__INTRANET__DEFAULT__SESSION", "SID=stale")];
    let waiting = itda(
        &home,
        &["call", "intranet.post-notice", "--title", "hi", "--json"],
        &expired,
    )
    .await;
    assert_eq!(waiting.status.code(), Some(20), "{waiting:?}");
    let id = stdout_json(&waiting)["approval"]
        .as_str()
        .unwrap()
        .to_owned();
    store.claim(&id).await.unwrap();
    let refused = itda(
        &home,
        &[
            "call",
            "intranet.post-notice",
            "--title",
            "hi",
            "--approval",
            &id,
            "--json",
        ],
        &expired,
    )
    .await;
    assert!(!refused.status.success(), "{refused:?}");
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_redirected_to_the_sign_in_page_means_the_session_expired() {
    use axum::response::Redirect;
    let home = home();
    let posts = Arc::new(AtomicUsize::new(0));
    let sent = posts.clone();
    let router = Router::new().route(
        "/items",
        axum::routing::post(move || {
            let sent = sent.clone();
            async move {
                sent.fetch_add(1, Ordering::SeqCst);
                Redirect::to("/user/login?redirect_to=%2Fitems")
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    install(&home, &base);

    let waiting = itda(&home, &["call", "fixture.create-item", "--json"], &[]).await;
    let id = stdout_json(&waiting)["approval"]
        .as_str()
        .unwrap()
        .to_owned();
    let store =
        itda_policy::Approvals::open(itda_store::Store::open(&home.join("itda.db")).unwrap())
            .unwrap();
    store.claim(&id).await.unwrap();
    let answered = itda(
        &home,
        &["call", "fixture.create-item", "--approval", &id, "--json"],
        &[],
    )
    .await;
    assert_eq!(answered.status.code(), Some(10), "{answered:?}");
    assert_eq!(stdout_json(&answered)["status"], "auth_required");
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_names_the_sign_in_and_signs_in_only_where_the_session_can_be_kept() {
    let home = home();
    let posts = Arc::new(AtomicUsize::new(0));
    let base = serve_intranet(posts).await;
    let url = format!("{base}/notices");
    let english = [("ITDA_LANG", "en")];

    // Without anyone to sign in, the redirect is named, not reported as a bad address.
    let refused = itda(
        &home,
        &[
            "setup",
            &url,
            "--name",
            "intranet",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &english,
    )
    .await;
    assert!(!refused.status.success());
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(said.contains("sends itda to a sign-in page"), "{said}");
    assert!(!said.contains("bad_url"), "{said}");

    // With a user name, the password is read from stdin; where secrets are
    // read-only (the tests' environment store) nothing is sent anywhere.
    let mut command = Command::new(env!("CARGO_BIN_EXE_itda"));
    command
        .args([
            "setup",
            &url,
            "--name",
            "intranet",
            "--user",
            "lab",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ])
        .env("ITDA_HOME", &home)
        .env("ITDA_ALLOW_LOCAL", "1")
        .env("ITDA_SECRET_STORE", "env")
        .env("ITDA_LANG", "en")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"secret-word\n")
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("read-only"), "{said}");
    assert!(!said.contains("secret-word"));
    assert!(!home.join("specs/intranet").exists());

    // `itda login` takes the same path and stops the same way.
    install_intranet(&home, &base);
    let mut command = Command::new(env!("CARGO_BIN_EXE_itda"));
    command
        .args(["login", "intranet", "--user", "lab"])
        .env("ITDA_HOME", &home)
        .env("ITDA_ALLOW_LOCAL", "1")
        .env("ITDA_SECRET_STORE", "env")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"secret-word\n")
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("read-only"));
    // An API key still goes in by name when asked for.
    let key = itda(&home, &["login", "intranet", "--secret", "api-key"], &[]).await;
    assert!(String::from_utf8_lossy(&key.stderr).contains("No secret was given"));
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_framed_portal_is_drafted_through_its_same_origin_frames() {
    use axum::response::Html;
    let home = home();
    let router = Router::new()
        .route(
            "/",
            get(|| async {
                Html(
                    "<title>Portal</title>
                     <iframe src='/frame/board' name='main'></iframe>
                     <iframe src='https://cdn.example.net/w'></iframe>",
                )
            }),
        )
        .route(
            "/frame/board",
            get(|| async {
                Html(
                    "<title>Board</title><h1>Notices</h1>
                     <table><thead><tr><th>No</th><th>Title</th></tr></thead>
                     <tr><td>2</td><td><a href='/n/2'>Second</a></td></tr>
                     <tr><td>1</td><td><a href='/n/1'>First</a></td></tr></table>",
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    // Before: the shell alone is a title check. The draft follows the frame
    // and reads the board inside it, under the frame's own address.
    let en = [("ITDA_LANG", "en")];
    let printed = itda(&home, &["draft", "page", &format!("{base}/")], &en).await;
    assert!(printed.status.success(), "{printed:?}");
    let yaml = String::from_utf8_lossy(&printed.stdout);
    assert!(yaml.contains("path: /frame/board"), "{yaml}");
    assert!(yaml.contains("items: table tbody tr"), "{yaml}");
    let said = String::from_utf8_lossy(&printed.stderr);
    assert!(said.contains("Followed 1 frame: /frame/board."), "{said}");

    let installed = itda(
        &home,
        &[
            "draft",
            "page",
            &format!("{base}/"),
            "--name",
            "portal",
            "--install",
        ],
        &[],
    )
    .await;
    assert!(installed.status.success(), "{installed:?}");
    let called = itda(&home, &["call", "portal.get-board", "--json"], &[]).await;
    assert!(called.status.success(), "{called:?}");
    assert_eq!(stdout_json(&called)["data"]["items"][0]["title"], "Second");
    std::fs::remove_dir_all(home).ok();
}

/// A shell whose same-origin frame answers with a 303 to a sign-in form, the
/// shape of the `framed` lab intranet. `draft page` names the frame as
/// leading to a sign-in, and `setup` reaches the sign-in instead of drafting
/// only the shell's title: without a user name it names the sign-in, and with
/// one it signs in through the form (stopped here only where the session
/// cannot be kept, before any password is sent). The records-drafting proof,
/// which needs a writable session store, is the lab test
/// `framed_setup_from_the_shell_reaches_the_board`.
async fn serve_framed_login() -> String {
    use axum::{
        http::{HeaderMap, StatusCode},
        response::{Html, IntoResponse, Redirect},
    };
    fn signed_in(headers: &HeaderMap) -> bool {
        headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.contains("SID=lab"))
    }
    let router = Router::new()
        .route(
            "/",
            get(|| async {
                Html("<title>Portal</title><iframe src='/frame/board' name='main'></iframe>")
            }),
        )
        .route(
            "/frame/board",
            get(|headers: HeaderMap| async move {
                if !signed_in(&headers) {
                    return Redirect::to("/frame/login?next=%2Fframe%2Fboard").into_response();
                }
                Html(
                    "<title>Board</title><h1>Notices</h1>
                     <table><thead><tr><th>No</th><th>Title</th></tr></thead>
                     <tr class='notice'><td>2</td><td><a href='/n/2'>Second</a></td></tr>
                     <tr class='notice'><td>1</td><td><a href='/n/1'>First</a></td></tr></table>",
                )
                .into_response()
            }),
        )
        .route(
            "/frame/login",
            get(|| async {
                Html("<title>Sign in</title><form method='post' action='/frame/login'><input type='hidden' name='next' value='/frame/board'><input name='username'><input type='password' name='password'></form>")
            })
            .post(|| async {
                ([("set-cookie", "SID=lab; Path=/")], Redirect::to("/frame/board"))
            }),
        )
        .route(
            "/n/2",
            get(|headers: HeaderMap| async move {
                if !signed_in(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                Html("<title>Second</title><article><h1>Second</h1><p>body</p></article>")
                    .into_response()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test(flavor = "multi_thread")]
async fn draft_page_names_a_frame_that_leads_to_a_sign_in() {
    let home = home();
    let base = serve_framed_login().await;
    let en = [("ITDA_LANG", "en")];
    let printed = itda(&home, &["draft", "page", &format!("{base}/")], &en).await;
    assert!(printed.status.success(), "{printed:?}");
    // The frame was not read, but the note says it leads to a sign-in rather
    // than only that it was not read.
    let said = String::from_utf8_lossy(&printed.stderr);
    assert!(
        said.contains("/frame/board in a frame that answers with a sign-in"),
        "{said}"
    );
    assert!(said.contains("itda setup --user"), "{said}");
    std::fs::remove_dir_all(home).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_signs_in_from_a_shell_frame_that_redirects_to_a_login() {
    let home = home();
    let base = serve_framed_login().await;
    let english = [("ITDA_LANG", "en")];

    // Without anyone to sign in, the shell no longer drafts only its title:
    // the frame's redirect to a sign-in is named, the way the top-level
    // page's would be.
    let refused = itda(
        &home,
        &[
            "setup",
            &format!("{base}/"),
            "--name",
            "framed",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        &english,
    )
    .await;
    assert!(!refused.status.success());
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(said.contains("sends itda to a sign-in page"), "{said}");
    assert!(said.contains("/frame/board"), "{said}");

    // With a user name, the password is read from stdin; where secrets are
    // read-only (the tests' environment store) nothing is sent anywhere,
    // which proves the frame's redirect reached the sign-in path.
    let mut command = Command::new(env!("CARGO_BIN_EXE_itda"));
    command
        .args([
            "setup",
            &format!("{base}/"),
            "--name",
            "framed",
            "--user",
            "lab",
            "--no-record",
            "--client",
            "none",
            "--yes",
        ])
        .env("ITDA_HOME", &home)
        .env("ITDA_ALLOW_LOCAL", "1")
        .env("ITDA_SECRET_STORE", "env")
        .env("ITDA_LANG", "en")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"secret-word\n")
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(said.contains("read-only"), "{said}");
    assert!(!said.contains("secret-word"));
    assert!(!home.join("specs/framed").exists());
    std::fs::remove_dir_all(home).ok();
}

fn install_query_session(home: &Path, base: &str) {
    let dir = home.join("specs/portal");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("spec.yaml"),
        format!(
            "spec_version: 1
site: {{name: portal, base_url: {base}}}
auth:
  - name: session
    kind: browser
    value_from: query:sid
    login: {{path: /login, user: user_id, password: pw}}
operations:
  - name: list-notices
    method: GET
    path: /api/notices
    auth: session
    effect: read
    evidence: doc
    response: {{format: json, required_pointers: [/items]}}
  - name: post-notice
    method: POST
    path: /api/notices
    auth: session
    effect: write
    evidence: doc
    body:
      form:
        title: {{type: string, required: true}}
"
        ),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_in_the_query_is_sent_and_kept_out_of_results_and_the_audit_log() {
    use axum::extract::Query;
    use axum::response::{IntoResponse, Redirect};
    const TOKEN: &str = "9f8e7d6c5b4a3210fedc";
    let home = home();
    let seen = Arc::new(AtomicUsize::new(0));
    let hits = seen.clone();
    let router = Router::new().route(
        "/api/notices",
        get(
            move |Query(q): Query<std::collections::HashMap<String, String>>| {
                let hits = hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    if q.get("sid").map(String::as_str) != Some(TOKEN) {
                        return Redirect::to("/login?next=%2Fapi%2Fnotices").into_response();
                    }
                    axum::Json(json!({"items": [{"id": 1, "title": "first"}]})).into_response()
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    install_query_session(&home, &base);
    let session = [("ITDA_SECRET__PORTAL__DEFAULT__SESSION", TOKEN)];

    // The session goes on the query; the answer comes back and the token is
    // nowhere in the result.
    let read = itda(&home, &["call", "portal.list-notices", "--json"], &session).await;
    assert!(read.status.success(), "{read:?}");
    let out = String::from_utf8_lossy(&read.stdout);
    assert_eq!(stdout_json(&read)["data"]["items"][0]["title"], "first");
    assert!(
        !out.contains(TOKEN),
        "the token is not in the result: {out}"
    );

    // A signed-out session (no token) is refused the way any sign-in is, and
    // the error text carries no token.
    let out = itda(&home, &["call", "portal.list-notices", "--json"], &[]).await;
    assert_eq!(out.status.code(), Some(10), "{out:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!text.contains(TOKEN), "{text}");

    // A change logs to the audit, and the token never lands there.
    let waiting = itda(
        &home,
        &["call", "portal.post-notice", "--title", "hi", "--json"],
        &session,
    )
    .await;
    let id = stdout_json(&waiting)["approval"]
        .as_str()
        .unwrap()
        .to_owned();
    let store =
        itda_policy::Approvals::open(itda_store::Store::open(&home.join("itda.db")).unwrap())
            .unwrap();
    store.claim(&id).await.unwrap();
    let _ = itda(
        &home,
        &[
            "call",
            "portal.post-notice",
            "--title",
            "hi",
            "--approval",
            &id,
            "--json",
        ],
        &session,
    )
    .await;
    let audit = itda(&home, &["audit", "--json"], &[]).await;
    let log = String::from_utf8_lossy(&audit.stdout);
    assert!(
        !log.contains(TOKEN),
        "the audit log carries no token: {log}"
    );
    assert!(seen.load(Ordering::SeqCst) >= 1);
    std::fs::remove_dir_all(home).ok();
}

/// A site whose attachment is a file to save, and a form that takes one.
fn install_files(home: &Path, base: &str) {
    let dir = home.join("specs/files");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("spec.yaml"),
        format!(
            "spec_version: 1
site:
  name: files
  base_url: {base}
operations:
  - name: get-attachment
    desc: Download an attachment
    method: GET
    path: /attachment.pdf
    effect: read
    evidence: doc
    response: {{format: file, media_types: [application/pdf]}}
  - name: attach
    desc: Attach a file
    method: POST
    path: /upload
    effect: write
    evidence: doc
    body:
      multipart:
        fields: {{title: {{type: string, required: true}}}}
        file: {{name: upload, from: attachment}}
"
        ),
    )
    .unwrap();
}

async fn serve_files() -> String {
    let router = Router::new().route(
        "/attachment.pdf",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "application/pdf")],
                "%PDF-1.4 cli fixture",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_read_saves_under_the_data_directory_or_out() {
    let home = home();
    install_files(&home, &serve_files().await);
    let english = [("ITDA_LANG", "en")];

    let text = String::from_utf8_lossy(&itda(&home, &["describe", "files"], &english).await.stdout)
        .into_owned();
    assert!(
        text.contains("returns a file: application/pdf, saved under"),
        "{text}"
    );
    assert!(text.contains("sends the file at attachment"), "{text}");
    let described =
        stdout_json(&itda(&home, &["describe", "files.get-attachment", "--json"], &[]).await);
    assert_eq!(described[0]["returns"]["file"]["sha256"], "string");

    let saved = itda(&home, &["call", "files.get-attachment", "--json"], &english).await;
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let envelope = stdout_json(&saved);
    let expected = home.join("files").join("files").join("attachment.pdf");
    assert_eq!(
        envelope["data"]["path"],
        expected.to_string_lossy().as_ref()
    );
    assert_eq!(envelope["data"]["bytes"], 20);
    assert_eq!(envelope["data"]["media_type"], "application/pdf");
    assert_eq!(
        envelope["data"]["sha256"],
        itda_types::util::hash(b"%PDF-1.4 cli fixture")
    );
    assert_eq!(std::fs::read(&expected).unwrap(), b"%PDF-1.4 cli fixture");
    let said = String::from_utf8_lossy(&saved.stderr).into_owned();
    assert!(
        said.starts_with(&format!(
            "Saved {} (20 bytes, sha256 ",
            expected.to_string_lossy()
        )),
        "{said}"
    );

    let out = home.join("downloads");
    let elsewhere = itda(
        &home,
        &[
            "call",
            "files.get-attachment",
            "--out",
            out.to_str().unwrap(),
        ],
        &english,
    )
    .await;
    assert!(elsewhere.status.success());
    assert!(out.join("attachment.pdf").exists());

    // The check never fills the folder: a file read is skipped by name.
    let checked = stdout_json(&itda(&home, &["check", "files", "--json"], &[]).await);
    let attachment = checked["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["action"] == "get-attachment")
        .unwrap();
    assert_eq!(attachment["status"], "skipped");
    assert_eq!(attachment["detail"], "saves a file");

    // An upload names the file in the approval; a missing file never asks.
    let missing = itda(
        &home,
        &[
            "call",
            "files.attach",
            "--title",
            "x",
            "--attachment",
            "/nowhere/plan.csv",
            "--json",
        ],
        &english,
    )
    .await;
    assert_eq!(missing.status.code(), Some(2));
    let plan = home.join("plan.csv");
    std::fs::write(&plan, b"a,b\n").unwrap();
    let asked = stdout_json(
        &itda(
            &home,
            &[
                "call",
                "files.attach",
                "--title",
                "x",
                "--attachment",
                plan.to_str().unwrap(),
                "--json",
            ],
            &english,
        )
        .await,
    );
    assert_eq!(asked["status"], "approval_required");
    let listed = stdout_json(&itda(&home, &["approvals", "--json"], &[]).await);
    let inputs = listed[0]["inputs"].as_array().unwrap();
    let file = inputs.iter().find(|i| i["label"] == "file").unwrap();
    assert_eq!(
        file["value"],
        format!(
            "plan.csv (4 bytes, sha256 {})",
            itda_types::util::hash(b"a,b\n")
        )
    );
    std::fs::remove_dir_all(home).ok();
}
