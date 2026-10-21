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
            ("call", "approval_expired"),
            ("rejected", "ok")
        ]
    );
    let verified = stdout_json(&itda(&home, &["audit", "--verify", "--json"], &[]).await);
    assert_eq!(verified["records"], 3);
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
    assert!(String::from_utf8_lossy(&printed.stderr).contains("3 actions (2 reads, 1 changes)"));

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
    assert_eq!(signed_in.status.code(), Some(0));
    let text = String::from_utf8_lossy(&signed_in.stdout);
    assert!(
        text.contains("verdict  ok: every read still matches"),
        "{text}"
    );
    assert!(text.contains("2 ok, 0 failed, 1 skipped."), "{text}");
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
    std::fs::remove_dir_all(home).ok();
}
