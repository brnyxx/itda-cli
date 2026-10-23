use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Output,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn browser() -> Option<String> {
    let browser = std::env::var("ITDA_TEST_BROWSER")
        .ok()
        .filter(|value| !value.is_empty());
    if browser.is_none() {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
    }
    browser
}

/// Stops the worker and removes the fixture home when a test fails before
/// its own `cleanup`; after `cleanup` there is nothing left to do.
struct Guard {
    home: PathBuf,
    browser: String,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.home.exists() {
            return;
        }
        let _ = std::process::Command::new(env!("CARGO_BIN_EXE_itda"))
            .arg("__browser-stop")
            .env("ITDA_HOME", &self.home)
            .env("ITDA_TRANSPORT", "auto")
            .env("ITDA_ALLOW_LOCAL", "1")
            .env("ITDA_SECRET_STORE", "env")
            .env("ITDA_BROWSER", &self.browser)
            .status();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn home() -> PathBuf {
    let home = std::env::temp_dir().join(format!("itda-adaptive-{}", itda_types::util::id()));
    std::fs::create_dir_all(&home).expect("create fixture home");
    home
}

fn install(home: &Path, name: &str, origin: &str) {
    let dir = home.join("specs").join(name);
    std::fs::create_dir_all(&dir).expect("create fixture site");
    let spec = json!({
        "spec_version":1,
        "site":{"name":name,"base_url":origin},
        "operations":[{
            "name":"items","method":"GET","path":"/items","effect":"read",
            "evidence":"local fixture", "query":{"q":{"type":"string"}},
            "response":{"format":"json","required_pointers":["/items"]},
        }],
    });
    std::fs::write(
        dir.join("spec.yaml"),
        serde_json::to_vec(&spec).expect("serialize fixture"),
    )
    .expect("install fixture");
}

async fn call(home: &Path, browser: &str, args: &[&str]) -> Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_itda"));
    command
        .args(args)
        .env("ITDA_HOME", home)
        .env("ITDA_TRANSPORT", "auto")
        .env("ITDA_ALLOW_LOCAL", "1")
        .env("ITDA_SECRET_STORE", "env")
        .env("ITDA_BROWSER", browser)
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .expect("CLI completed within deadline")
        .expect("CLI started")
}

async fn cleanup(home: &Path, browser: &str) {
    let result = call(home, browser, &["__browser-stop"]).await;
    assert!(result.status.success(), "{result:?}");
    std::fs::remove_dir_all(home).expect("remove fixture home");
}

#[tokio::test(flavor = "multi_thread")]
async fn separate_cli_processes_keep_the_session_when_inputs_change() {
    let Some(browser) = browser() else { return };
    let home = home();
    let _guard = Guard {
        home: home.clone(),
        browser: browser.clone(),
    };
    let direct = Arc::new(AtomicUsize::new(0));
    let sessions = Arc::new(AtomicUsize::new(0));
    let router = Router::new().route(
        "/items",
        get({
            let direct = direct.clone();
            let sessions = sessions.clone();
            move |headers: HeaderMap| {
                let direct = direct.clone();
                let sessions = sessions.clone();
                async move {
                    if !headers.contains_key("sec-fetch-mode") {
                        direct.fetch_add(1, Ordering::SeqCst);
                        return (StatusCode::FORBIDDEN, Html("<title>Just a moment</title>"))
                            .into_response();
                    }
                    let existing = headers
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| {
                            v.split("; ")
                                .find_map(|pair| pair.strip_prefix("generation="))
                        })
                        .and_then(|v| v.parse::<usize>().ok());
                    let generation =
                        existing.unwrap_or_else(|| sessions.fetch_add(1, Ordering::SeqCst) + 1);
                    (
                        [(
                            "set-cookie",
                            format!("generation={generation}; Path=/; HttpOnly"),
                        )],
                        axum::Json(json!({"items":[{"generation":generation}]})),
                    )
                        .into_response()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    install(
        &home,
        "fixture",
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    for query in ["one", "two"] {
        let result = call(
            &home,
            &browser,
            &["call", "fixture.items", "--q", query, "--json"],
        )
        .await;
        assert!(result.status.success(), "{result:?}");
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["data"]["items"][0]["generation"], 1);
    }
    assert_eq!(direct.load(Ordering::SeqCst), 1);
    assert_eq!(sessions.load(Ordering::SeqCst), 1);
    cleanup(&home, &browser).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn separate_origins_do_not_serialize_browser_execution() {
    let Some(browser) = browser() else { return };
    let home = home();
    let _guard = Guard {
        home: home.clone(),
        browser: browser.clone(),
    };
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut servers = Vec::new();
    for name in ["first", "second"] {
        let barrier = barrier.clone();
        let router = Router::new().route(
            "/items",
            get(move |headers: HeaderMap| {
                let barrier = barrier.clone();
                async move {
                    if !headers.contains_key("sec-fetch-mode") {
                        return (StatusCode::FORBIDDEN, Html("<title>Just a moment</title>"))
                            .into_response();
                    }
                    barrier.wait().await;
                    axum::Json(json!({"items":[{"title":"overlapped"}]})).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        install(
            &home,
            name,
            &format!("http://{}", listener.local_addr().unwrap()),
        );
        servers.push(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap()
        }));
    }
    let (first, second) = tokio::join!(
        call(&home, &browser, &["call", "first.items", "--json"]),
        call(&home, &browser, &["call", "second.items", "--json"]),
    );
    assert!(first.status.success(), "{first:?}");
    assert!(second.status.success(), "{second:?}");
    cleanup(&home, &browser).await;
    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_crashed_broker_is_replaced_and_its_owned_profile_is_reaped() {
    let Some(browser) = browser() else { return };
    let home = home();
    let _guard = Guard {
        home: home.clone(),
        browser: browser.clone(),
    };
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = seen.clone();
    let router = Router::new().route(
        "/items",
        get(move |headers: HeaderMap| {
            let seen = requests.clone();
            async move {
                seen.lock().unwrap().push(
                    headers
                        .get("sec-fetch-mode")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("direct")
                        .to_owned(),
                );
                if !headers.contains_key("sec-fetch-mode") {
                    return (StatusCode::FORBIDDEN, Html("<title>Just a moment</title>"))
                        .into_response();
                }
                axum::Json(json!({"items":[{"title":"recovered"}]})).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    install(
        &home,
        "fixture",
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let first = call(&home, &browser, &["call", "fixture.items", "--json"]).await;
    assert!(first.status.success(), "{first:?}");
    let namespace = std::fs::read_dir(home.join("browser-runtime"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.join("endpoint.json").exists())
        .unwrap();
    let record_path = std::fs::read_dir(namespace.join("chrome"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .unwrap();
    let record: Value = serde_json::from_slice(&std::fs::read(record_path).unwrap()).unwrap();
    let owner = record["owner_pid"].as_u64().unwrap().to_string();
    let old_profile = PathBuf::from(record["profile"].as_str().unwrap());
    assert!(old_profile.starts_with(namespace.join("chrome")));
    let killed = tokio::process::Command::new("/bin/kill")
        .args(["-KILL", &owner])
        .status()
        .await
        .unwrap();
    assert!(killed.success());
    let lock = namespace.join("owner.lock");
    let released = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock)
                .unwrap();
            file.lock().unwrap();
            file
        }),
    )
    .await
    .unwrap()
    .unwrap();
    drop(released);
    let recovered = call(&home, &browser, &["call", "fixture.items", "--json"]).await;
    assert!(
        recovered.status.success(),
        "{recovered:?}; requests: {:?}",
        seen.lock().unwrap()
    );
    assert!(!old_profile.exists());
    cleanup(&home, &browser).await;
    server.abort();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_broker_crash_during_browser_startup_is_recovered_in_the_same_call() {
    use std::{
        io::{BufRead, Read},
        os::unix::fs::PermissionsExt,
    };
    let Some(real_browser) = browser() else {
        return;
    };
    let home = home();
    let ready = home.join("ready");
    let release = home.join("release");
    assert!(
        tokio::process::Command::new("mkfifo")
            .arg(&ready)
            .arg(&release)
            .status()
            .await
            .unwrap()
            .success()
    );
    let wrapper = home.join("browser");
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\nfor arg in \"$@\"; do\n if [ \"$arg\" = --version ]; then exec '{real_browser}' \"$@\"; fi\ndone\nif mkdir '{}' 2>/dev/null; then\n exec 3>'{}'\n printf '%s\\n' \"$$\" >&3\n read -r proceed <'{}'\nfi\nexec '{real_browser}' \"$@\"\n",
        home.join("once").display(), ready.display(), release.display(),
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _guard = Guard {
        home: home.clone(),
        browser: wrapper.to_str().unwrap().to_owned(),
    };
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut exited = tokio::task::spawn_blocking(move || {
        let mut pipe = std::io::BufReader::new(std::fs::File::open(ready).unwrap());
        let mut line = String::new();
        pipe.read_line(&mut line).unwrap();
        started_tx
            .send(line.trim().parse::<u32>().unwrap())
            .unwrap();
        pipe.read_to_end(&mut Vec::new()).unwrap();
    });
    let router = Router::new().route(
        "/items",
        get(|headers: HeaderMap| async move {
            if !headers.contains_key("sec-fetch-mode") {
                return (StatusCode::FORBIDDEN, Html("<title>Just a moment</title>"))
                    .into_response();
            }
            axum::Json(json!({"items":[{"title":"recovered"}]})).into_response()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    install(
        &home,
        "fixture",
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let request = {
        let home = home.clone();
        let wrapper = wrapper.clone();
        tokio::spawn(async move {
            call(
                &home,
                wrapper.to_str().unwrap(),
                &["call", "fixture.items", "--json"],
            )
            .await
        })
    };
    let launcher = tokio::time::timeout(Duration::from_secs(20), started_rx)
        .await
        .unwrap()
        .unwrap();
    let namespace = std::fs::read_dir(home.join("browser-runtime"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.join("endpoint.json").exists())
        .unwrap();
    let endpoint: Value =
        serde_json::from_slice(&std::fs::read(namespace.join("endpoint.json")).unwrap()).unwrap();
    let owner = endpoint["owner_pid"].as_u64().unwrap();
    let profile = std::fs::read_dir(namespace.join("chrome/profiles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let guard = std::fs::read_dir(namespace.join("chrome/guardians"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let guard = std::fs::File::open(guard).unwrap();
    let reaped = tokio::task::spawn_blocking(move || {
        guard.lock().unwrap();
        !profile.exists()
    });
    assert!(owner > 1);
    assert!(
        tokio::process::Command::new("/bin/kill")
            .args(["-KILL", &owner.to_string()])
            .status()
            .await
            .unwrap()
            .success()
    );
    let cleaned = tokio::time::timeout(Duration::from_secs(5), &mut exited)
        .await
        .is_ok();
    if !cleaned {
        let _ = tokio::process::Command::new("/bin/kill")
            .args(["-KILL", &launcher.to_string()])
            .status()
            .await;
        exited.await.unwrap();
    }
    let response = request.await.unwrap();
    let reaped = tokio::time::timeout(Duration::from_secs(5), reaped)
        .await
        .unwrap()
        .unwrap();
    cleanup(&home, wrapper.to_str().unwrap()).await;
    server.abort();
    assert!(
        cleaned,
        "browser launcher survived its broker before a CDP record existed"
    );
    assert!(reaped, "the startup profile survived its supervisor");
    assert!(response.status.success(), "{response:?}");
}
