//! A site whose only read saves a file, against a local fixture. Setup tries
//! it once by its headers and first bytes and keeps nothing on disk; `itda
//! check` never fetches it.
#![allow(clippy::unwrap_used)]
use axum::{Router, routing::get};
use itda_net::{Gate, Network};
use itda_runtime::Runtime;
use itda_setup::check;
use itda_store::Store;
use itda_vault::{MemoryStore, Vault};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn serve(hits: Arc<AtomicUsize>) -> String {
    let pdf = hits.clone();
    let router = Router::new()
        .route(
            "/files/minutes.pdf",
            get(move || {
                pdf.fetch_add(1, Ordering::SeqCst);
                async {
                    let mut body = b"%PDF-1.4 minutes ".to_vec();
                    body.resize(512 * 1024, b' ');
                    ([("content-type", "application/pdf")], body)
                }
            }),
        )
        .route(
            "/files/report.pdf",
            get(move || {
                hits.fetch_add(1, Ordering::SeqCst);
                async { axum::response::Html("<html><title>Sign in</title></html>") }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn only_an_attachment(base: &str, path: &str) -> itda_spec::Spec {
    let yaml = format!(
        "spec_version: 1
site: {{name: board, base_url: {base}}}
operations:
  - name: get-minutes
    desc: Download an attachment from Board
    method: GET
    path: {path}
    effect: read
    evidence: the board links the minutes as an attachment
    response: {{format: file, media_types: [application/pdf]}}
"
    );
    itda_spec::parse(yaml.as_bytes()).unwrap()
}

fn runtime(files: &std::path::Path) -> Runtime {
    let store = Store::in_memory().unwrap();
    let gate = Arc::new(Gate::with_limits(store.clone(), 0, 1000).unwrap());
    let vault = Arc::new(Vault::open(store, Arc::new(MemoryStore::default())).unwrap());
    Runtime::new(Arc::new(Network::new(gate, true)), vault).with_files(files)
}

#[tokio::test]
async fn setup_tries_a_file_read_once_and_saves_nothing() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(hits.clone()).await;
    let files = std::env::temp_dir().join(format!("itda-setup-files-{}", itda_types::util::id()));
    let runtime = runtime(&files);
    let spec = only_an_attachment(&base, "/files/minutes.pdf");

    let tried = check::try_once(&runtime, &spec).await;
    assert_eq!(tried[0].status, "ok", "{tried:?}");
    assert!(!check::nothing_worked(&spec, &tried), "{tried:?}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let saved = std::fs::read_dir(&files)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(saved, 0, "setup kept a file under {}", files.display());

    // A check never fills a folder: the file read stays skipped.
    let checked = check::run(&runtime, std::slice::from_ref(&spec), None).await;
    assert_eq!(checked[0].status, "skipped");
    assert_eq!(checked[0].detail, "saves a file");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(&files).ok();
}

#[tokio::test]
async fn a_file_read_that_answers_with_a_page_is_not_kept() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = serve(hits.clone()).await;
    let files = std::env::temp_dir().join(format!("itda-setup-files-{}", itda_types::util::id()));
    let spec = only_an_attachment(&base, "/files/report.pdf");

    let tried = check::try_once(&runtime(&files), &spec).await;
    assert_eq!(tried[0].status, "wrong_media_type", "{tried:?}");
    assert!(check::nothing_worked(&spec, &tried));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(&files).ok();
}
