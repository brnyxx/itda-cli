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
        [
            "fixture.create-item",
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
            "{base}/openapi.json의 API 설명을 읽었습니다: 읽기 1개, 변경 1개는 메모로 남김."
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
            "note: {base}/openapi.json could not be read as an API description"
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
    assert!(err.contains("itda does not go around that"), "{err}");
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
    assert!(said.contains("No sources are known"), "{said}");
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
