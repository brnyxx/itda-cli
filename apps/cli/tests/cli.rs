//! Runs the built `itda` binary against a synthetic local server and a
//! temporary data directory. Nothing here contacts a real website or the
//! system keyring.
#![allow(clippy::unwrap_used)]
use axum::{Router, http::HeaderMap, routing::get};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
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
        );
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
        .envs(env.iter().copied());
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
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
    assert_eq!(effects, ["read", "write", "read"]);
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

#[test]
fn the_sandbox_definition_is_valid() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sandbox/spec.yaml");
    let spec = itda_spec::parse(&std::fs::read(path).unwrap()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();
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
        ["fixture.create-item", "fixture.list-items", "fixture.me"]
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
    assert!(String::from_utf8_lossy(&piped.stdout).contains("POST"));
    let early = itda(
        &home,
        &["call", "fixture.create-item", "--approval", &id, "--json"],
        &[],
    )
    .await;
    assert_eq!(early.status.code(), Some(21));

    assert!(itda(&home, &["reject", &id], &[]).await.status.success());
    let none = itda(&home, &["approvals"], &[]).await;
    assert!(String::from_utf8_lossy(&none.stdout).contains("No changes"));
    std::fs::remove_dir_all(home).ok();
}
