//! Runs the built `itda` binary against the lab in `examples/lab`: real
//! self-hosted software behind a sign-in (Gitea, Redmine, WordPress,
//! MediaWiki and a small legacy intranet), each with the account
//! `lab` / `labpassword`. Only with `ITDA_LAB=1` and the lab up
//! (`examples/lab/lab.sh up`); otherwise every test says it skipped.
//!
//! Per service: `itda setup <page> --user lab` signs in through the site's
//! own form and drafts from the signed-in page; the reviewed definition from
//! `examples/lab/specs` is installed; `itda login <site> --user lab` signs in
//! again; a read answers; a change waits for an approval, which is given in a
//! terminal (through `expect`); the change lands; the audit log verifies.
//! Sessions are sealed under each test's data directory with a master key
//! in the real system keyring, which is removed at the end.
#![allow(clippy::unwrap_used)]
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const USER: &str = "lab";
const PASSWORD: &str = "labpassword";

struct Service {
    site: &'static str,
    /// The page a person would give `itda setup`.
    page: &'static str,
    read: &'static str,
    change: &'static str,
    /// The change's inputs, with `{title}` filled in.
    inputs: &'static [(&'static str, &'static str)],
}

const SERVICES: [Service; 5] = [
    Service {
        site: "gitea",
        page: "http://127.0.0.1:8081/issues",
        read: "list-issues",
        change: "create-issue",
        inputs: &[("title", "{title}"), ("content", "opened by the lab test")],
    },
    Service {
        site: "redmine",
        page: "http://127.0.0.1:8082/projects/lab/issues",
        read: "list-issues",
        change: "create-issue",
        inputs: &[("issue[subject]", "{title}")],
    },
    Service {
        site: "wordpress",
        page: "http://127.0.0.1:8083/wp-admin/edit.php",
        read: "list-private-posts",
        change: "create-post",
        inputs: &[("title", "{title}"), ("content", "written by the lab test")],
    },
    Service {
        site: "mediawiki",
        page: "http://127.0.0.1:8084/index.php/Special:AllPages",
        read: "list-pages",
        change: "edit-page",
        inputs: &[("title", "{title}"), ("text", "written by the lab test")],
    },
    Service {
        site: "intranet",
        page: "http://127.0.0.1:8085/notices",
        read: "list-notices",
        change: "post-notice",
        inputs: &[("title", "{title}"), ("body", "written by the lab test")],
    },
];

fn lab_is_on() -> bool {
    let on = std::env::var("ITDA_LAB").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set ITDA_LAB=1 with examples/lab up to run");
    }
    on
}

fn home() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("itda-lab-{}", itda_types::util::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn spec_path(site: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/lab/specs")
        .join(format!("{site}.yaml"))
}

fn command(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_itda"));
    command
        .args(args)
        .env("ITDA_HOME", home)
        .env("ITDA_ALLOW_LOCAL", "1")
        .env("ITDA_LANG", "en")
        .env_remove("ITDA_SECRET_STORE");
    command
}

fn itda(home: &Path, args: &[&str]) -> Output {
    command(home, args).output().unwrap()
}

/// Run with `line` as stdin, the way a password manager pipes a password.
fn itda_with_stdin(home: &Path, args: &[&str], line: &str) -> Output {
    use std::io::Write;
    let mut child = command(home, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{line}\n").as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "not JSON: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// `itda approve` only takes its answer from a terminal, so the yes goes
/// through a pseudo-terminal that `expect` drives.
fn approve_in_a_terminal(home: &Path, id: &str) -> String {
    let script = format!(
        "spawn -noecho {} approve {id}\nexpect \"y/N\"\nsend \"y\\r\"\nexpect eof",
        env!("CARGO_BIN_EXE_itda")
    );
    let output = Command::new("expect")
        .arg("-c")
        .arg(script)
        .env("ITDA_HOME", home)
        .env("ITDA_ALLOW_LOCAL", "1")
        .env("ITDA_LANG", "en")
        .env_remove("ITDA_SECRET_STORE")
        .output()
        .expect("the lab test approves changes through `expect`, which ships with macOS");
    text(&output)
}

/// The sealed store keeps one master key per data directory in the system
/// keyring, named after a hash of the directory; a test's directory is
/// thrown away, so its key goes too. On macOS a key left by another build
/// of itda cannot be rewritten without a prompt, which is the other reason.
fn forget_master_key(home: &Path) {
    if cfg!(target_os = "macos") {
        let tag: String = itda_types::util::hash(home.to_string_lossy().as_bytes())
            .chars()
            .take(16)
            .collect();
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                "itda",
                "-a",
                &format!("itda/vault/master-key-{tag}"),
            ])
            .output();
    }
}

fn inputs(service: &Service, title: &str) -> Vec<String> {
    service
        .inputs
        .iter()
        .flat_map(|(name, value)| [format!("--{name}"), value.replace("{title}", title)])
        .collect()
}

/// The whole path, start to finish, for one service.
fn run(service: &Service) {
    let home = home();
    let site = service.site;

    // setup: the page needs a sign-in, itda signs in through the form.
    let setup = itda_with_stdin(
        &home,
        &[
            "setup",
            service.page,
            "--name",
            site,
            "--user",
            USER,
            "--no-record",
            "--client",
            "none",
            "--yes",
        ],
        PASSWORD,
    );
    let said = text(&setup);
    assert!(setup.status.success(), "setup failed:\n{said}");
    assert!(said.contains("accepted the sign-in"), "{said}");
    assert!(said.contains("Kept the session"), "{said}");
    let drafted = std::fs::read_to_string(home.join(format!("specs/{site}/spec.yaml"))).unwrap();
    assert!(drafted.contains("kind: browser"), "{drafted}");
    assert!(drafted.contains("login:"), "{drafted}");
    assert!(drafted.contains("auth: session"), "{drafted}");

    // The drafted reads answer with the session that was just kept.
    let listed = json(&itda(&home, &["ls", site, "--json"]));
    let drafted_read = listed[0]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["effect"] == "read")
        .and_then(|a| a["name"].as_str().map(str::to_owned));
    if let Some(read) = &drafted_read {
        let called = itda(&home, &["call", &format!("{site}.{read}"), "--json"]);
        assert!(called.status.success(), "{}", text(&called));
    }

    // The reviewed definition, then a fresh sign-in with `itda login`.
    let added = itda(
        &home,
        &["add", spec_path(site).to_str().unwrap(), "--force"],
    );
    assert!(added.status.success(), "{}", text(&added));
    assert!(itda(&home, &["logout", site]).status.success());
    let refused = itda_with_stdin(&home, &["login", site, "--user", USER], "not-the-password");
    assert_eq!(refused.status.code(), Some(10), "{}", text(&refused));
    let login = itda_with_stdin(&home, &["login", site, "--user", USER], PASSWORD);
    assert!(login.status.success(), "{}", text(&login));
    assert!(text(&login).contains("Signed in"), "{}", text(&login));
    assert!(!text(&login).contains(PASSWORD));

    // A read answers with records.
    let read = itda(
        &home,
        &["call", &format!("{site}.{}", service.read), "--json"],
    );
    assert!(read.status.success(), "read failed: {}", text(&read));
    let before = json(&read);
    assert!(
        before["data"].as_array().is_some_and(|a| !a.is_empty())
            || before["data"]["items"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
        "{before}"
    );

    // A change waits for the person, who approves it in a terminal.
    let title = format!("lab test {}", &itda_types::util::id()[..8]);
    let args = inputs(service, &title);
    let mut call = vec!["call".to_owned(), format!("{site}.{}", service.change)];
    call.extend(args.clone());
    call.push("--json".into());
    let call: Vec<&str> = call.iter().map(String::as_str).collect();
    let waiting = itda(&home, &call);
    assert_eq!(waiting.status.code(), Some(20), "{}", text(&waiting));
    let id = json(&waiting)["approval"].as_str().unwrap().to_owned();
    let approved = approve_in_a_terminal(&home, &id);
    assert!(approved.contains("Approved"), "{approved}");

    // The change lands, once.
    let mut with_approval = call.clone();
    with_approval.extend(["--approval", &id]);
    let landed = itda(&home, &with_approval);
    assert!(landed.status.success(), "change failed: {}", text(&landed));
    assert_eq!(json(&landed)["status"], "ok");
    let again = itda(&home, &with_approval);
    assert_eq!(again.status.code(), Some(21), "{}", text(&again));
    let after = itda(
        &home,
        &["call", &format!("{site}.{}", service.read), "--json"],
    );
    assert!(
        String::from_utf8_lossy(&after.stdout)
            .to_lowercase()
            .contains(&title.to_lowercase()),
        "the change did not land: {}",
        text(&after)
    );

    // Everything is in the audit log, and the chain verifies.
    let audit = json(&itda(&home, &["audit", "--verify", "--json"]));
    assert_eq!(audit["status"], "ok", "{audit}");
    let records = json(&itda(&home, &["audit", "--json"]));
    let kinds: Vec<(String, String)> = records
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["kind"].as_str().unwrap().to_owned(),
                r["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(
        kinds.contains(&("approved".into(), "ok".into())),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&("change_started".into(), "sending".into())),
        "{kinds:?}"
    );
    let check = itda(&home, &["check", site]);
    assert!(check.status.success(), "{}", text(&check));
    assert!(text(&check).contains("verdict  ok"), "{}", text(&check));

    let _ = itda(&home, &["logout", site]);
    forget_master_key(&home);
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn gitea_from_setup_to_an_approved_change() {
    if lab_is_on() {
        run(&SERVICES[0]);
    }
}

#[test]
fn redmine_from_setup_to_an_approved_change() {
    if lab_is_on() {
        run(&SERVICES[1]);
    }
}

#[test]
fn wordpress_from_setup_to_an_approved_change() {
    if lab_is_on() {
        run(&SERVICES[2]);
    }
}

#[test]
fn mediawiki_from_setup_to_an_approved_change() {
    if lab_is_on() {
        run(&SERVICES[3]);
    }
}

#[test]
fn the_legacy_intranet_from_setup_to_an_approved_change() {
    if lab_is_on() {
        run(&SERVICES[4]);
    }
}

#[test]
fn the_lab_definitions_are_valid() {
    for service in &SERVICES {
        let bytes = std::fs::read(spec_path(service.site)).unwrap();
        let spec = itda_spec::parse(&bytes).unwrap();
        assert_eq!(spec.site.name, service.site);
        itda_policy::validate_spec(&spec).unwrap();
        assert!(spec.op(service.read).is_ok());
        assert!(spec.op(service.change).is_ok());
        let login = spec
            .auth
            .iter()
            .find_map(|auth| auth.login.as_ref())
            .unwrap();
        assert!(login.path.starts_with('/'));
    }
}
