//! Files in and files out against synthetic local servers. Nothing here
//! contacts a real website, and nothing waits on a sleep.
#![allow(clippy::unwrap_used)]
use axum::{
    Router,
    body::Bytes,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use itda_net::{Download, FileToSend, Gate, Multipart, Network, Request};
use itda_store::Store;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn host(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    address
}

fn network() -> Network {
    let store = Store::in_memory().unwrap();
    Network::new(Arc::new(Gate::with_limits(store, 0, 1000).unwrap()), true)
}

fn folder(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("itda-files-{tag}-{}", itda_types::util::id()))
}

fn download(dir: &Path, max_bytes: u64) -> Download {
    Download {
        dir: dir.to_path_buf(),
        max_bytes,
        media_types: vec!["application/pdf".into(), "text/csv".into()],
        allow_empty: false,
    }
}

fn read_file(url: String, dir: &Path, max_bytes: u64) -> Request {
    let mut request = Request::read(url);
    request.download = Some(download(dir, max_bytes));
    request
}

/// Nothing but the files named stays in the folder: no `.part` leftovers.
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

const PDF: &[u8] = b"%PDF-1.4 fixture bytes, not a real document";

fn fixtures() -> Router {
    let big: Bytes = Bytes::from(vec![b'x'; 300 * 1024]);
    Router::new()
        .route(
            "/report.pdf",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "application/pdf"),
                        (
                            header::CONTENT_DISPOSITION,
                            "attachment; filename=\"monthly report.pdf\"",
                        ),
                    ],
                    PDF,
                )
            }),
        )
        .route(
            "/big",
            get(move || {
                let big = big.clone();
                async move { ([(header::CONTENT_TYPE, "application/pdf")], big) }
            }),
        )
        .route(
            "/page",
            get(|| async { axum::response::Html("<html><body>Sign in</body></html>") }),
        )
        .route(
            "/inline.html",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "text/html"),
                        (header::CONTENT_DISPOSITION, "attachment; filename=a.html"),
                    ],
                    "<p>a saved page</p>",
                )
            }),
        )
        .route(
            "/part",
            get(|| async {
                (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        (header::CONTENT_TYPE, "application/pdf"),
                        (header::CONTENT_RANGE, "bytes 0-9/100"),
                    ],
                    &PDF[..10],
                )
            }),
        )
        .route(
            "/empty.csv",
            get(|| async { ([(header::CONTENT_TYPE, "text/csv")], "") }),
        )
        .route(
            "/denied",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({"code": "forbidden"})),
                )
            }),
        )
}

#[tokio::test]
async fn a_file_is_saved_sealed_and_named_with_its_hash() {
    let base = host(fixtures()).await;
    let dir = folder("ok");
    let net = network();
    let started = std::time::Instant::now();
    let response = net
        .fetch(read_file(format!("{base}/report.pdf"), &dir, 1024 * 1024))
        .await
        .unwrap();
    let took = started.elapsed();
    let saved = response.file.clone().unwrap();
    assert_eq!(saved.path, dir.join("monthly_report.pdf"));
    assert_eq!(saved.bytes, PDF.len() as u64);
    assert_eq!(saved.media_type, "application/pdf");
    assert_eq!(saved.sha256, itda_types::util::hash(PDF));
    assert_eq!(std::fs::read(&saved.path).unwrap(), PDF);
    assert!(response.bytes.is_empty());
    assert!(response.successful());
    eprintln!(
        "measured download: {} bytes in {} ms, sha256 {}",
        saved.bytes,
        took.as_millis(),
        saved.sha256
    );

    // The same name again never replaces the first file.
    let again = net
        .fetch(read_file(format!("{base}/report.pdf"), &dir, 1024 * 1024))
        .await
        .unwrap();
    assert_eq!(again.file.unwrap().path, dir.join("monthly_report-2.pdf"));
    assert_eq!(
        files_in(&dir),
        ["monthly_report-2.pdf", "monthly_report.pdf"]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_answer_over_the_cap_is_refused_and_leaves_nothing() {
    let base = host(fixtures()).await;
    let dir = folder("cap");
    let net = network();
    // Known length: refused from the headers alone.
    let error = net
        .fetch(read_file(format!("{base}/big"), &dir, 100 * 1024))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "too_large");
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
    // The buffered path keeps its own limit untouched by the file cap.
    let mut request = Request::read(format!("{base}/big"));
    request.limit = 10;
    assert_eq!(
        net.fetch(request).await.unwrap_err().public_code,
        "too_large"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_stream_over_the_cap_is_cut_and_the_part_removed() {
    // A raw server that sends no Content-Length, so the cap is hit mid-stream.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 4096];
        let _ = socket.read(&mut buffer).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        for _ in 0..40 {
            socket.write_all(b"400\r\n").await.unwrap();
            socket.write_all(&[b'y'; 0x400]).await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
        }
        socket.write_all(b"0\r\n\r\n").await.unwrap();
    });
    let dir = folder("stream-cap");
    let error = network()
        .fetch(read_file(format!("{base}/chunked.pdf"), &dir, 8 * 1024))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "too_large");
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_page_a_part_or_nothing_is_not_the_file() {
    let base = host(fixtures()).await;
    let dir = folder("not-file");
    let net = network();
    let why = |path: &'static str| {
        let url = format!("{base}/{path}");
        let dir = dir.clone();
        let net = &net;
        async move {
            net.fetch(read_file(url, &dir, 1024 * 1024))
                .await
                .unwrap_err()
                .public_code
        }
    };
    assert_eq!(why("page").await, "wrong_media_type");
    assert_eq!(why("part").await, "partial_answer");
    assert_eq!(why("empty.csv").await, "empty_answer");
    // A refusal is answered as the refusal it is, for the caller to name.
    let denied = net
        .fetch(read_file(format!("{base}/denied"), &dir, 1024))
        .await
        .unwrap();
    assert_eq!((denied.status, denied.file), (403, None));
    assert!(!denied.bytes.is_empty());
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));

    // A page the site marks as an attachment is a file; so is an empty
    // answer when the read allows one.
    let mut html = read_file(format!("{base}/inline.html"), &dir, 1024);
    html.download.as_mut().unwrap().media_types = vec!["text/html".into()];
    let saved = net.fetch(html).await.unwrap().file.unwrap();
    assert_eq!(saved.path, dir.join("a.html"));
    let mut empty = read_file(format!("{base}/empty.csv"), &dir, 1024);
    empty.download.as_mut().unwrap().allow_empty = true;
    let saved = net.fetch(empty).await.unwrap().file.unwrap();
    assert_eq!((saved.bytes, saved.path), (0, dir.join("empty.csv")));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_short_answer_is_partial_and_nothing_is_kept() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 4096];
        let _ = socket.read(&mut buffer).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: 100\r\nConnection: close\r\n\r\n%PDF-short",
            )
            .await
            .unwrap();
    });
    let dir = folder("short");
    let error = network()
        .fetch(read_file(format!("{base}/cut.pdf"), &dir, 1024))
        .await
        .unwrap_err();
    assert_eq!(error.public_code, "partial_answer");
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
    std::fs::remove_dir_all(&dir).ok();
}

/// What the upload fixture received: the headers and the whole body.
#[derive(Default)]
struct Received {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn upload_fixture(received: Arc<Mutex<Received>>) -> Router {
    Router::new().route(
        "/upload",
        post(move |headers: HeaderMap, body: Bytes| {
            let received = received.clone();
            async move {
                let mut got = received.lock().unwrap();
                got.headers = headers
                    .iter()
                    .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
                    .collect();
                got.body = body.to_vec();
                (StatusCode::CREATED, "ok").into_response()
            }
        }),
    )
}

fn upload(url: String, dir: &Path, bytes: &[u8], sha256: Option<String>) -> (Request, PathBuf) {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("plan.csv");
    std::fs::write(&path, bytes).unwrap();
    let (size, digest) = itda_types::util::file_digest(&path).unwrap();
    let mut request = Request::read(url);
    request.method = "POST".into();
    request.read_only = false;
    request.follow_redirects = false;
    request.multipart = Some(Multipart {
        fields: vec![("title".into(), "Q4 plan".into())],
        between_chunks: None,
        file: Some(FileToSend {
            field: "upload".into(),
            path: path.clone(),
            file_name: "plan.csv".into(),
            media_type: "text/csv".into(),
            bytes: size,
            sha256: sha256.unwrap_or(digest),
        }),
    });
    (request, path)
}

#[tokio::test]
async fn an_upload_streams_the_file_with_a_known_length() {
    let received = Arc::new(Mutex::new(Received::default()));
    let base = host(upload_fixture(received.clone())).await;
    let dir = folder("upload");
    let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let (request, _) = upload(format!("{base}/upload"), &dir, &content, None);
    let started = std::time::Instant::now();
    let response = network().fetch(request).await.unwrap();
    let took = started.elapsed();
    assert_eq!(response.status, 201);

    let got = received.lock().unwrap();
    let header = |name: &str| {
        got.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    };
    let content_type = header("content-type").to_owned();
    assert!(
        content_type.starts_with("multipart/form-data; boundary=itda-"),
        "{content_type}"
    );
    assert_eq!(header("content-length"), got.body.len().to_string());
    assert_eq!(header("transfer-encoding"), "");
    let boundary = content_type.rsplit('=').next().unwrap().to_owned();
    let body = &got.body;
    let text = String::from_utf8_lossy(&body[..600]).into_owned();
    assert!(text.starts_with(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nQ4 plan\r\n--{boundary}\r\n")), "{text}");
    assert!(text.contains("Content-Disposition: form-data; name=\"upload\"; filename=\"plan.csv\"\r\nContent-Type: text/csv\r\n\r\n"), "{text}");
    let start = body.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let start = start
        + 4
        + body[start + 4..]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap()
        + 4;
    let end = body.len() - format!("\r\n--{boundary}--\r\n").len();
    assert_eq!(&body[start..end], &content[..]);
    assert!(body.ends_with(format!("\r\n--{boundary}--\r\n").as_bytes()));
    eprintln!(
        "measured upload: {} bytes in {} ms, sha256 {}",
        content.len(),
        took.as_millis(),
        itda_types::util::hash(&content)
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_file_that_differs_from_the_approved_one_is_not_sent() {
    let received = Arc::new(Mutex::new(Received::default()));
    let base = host(upload_fixture(received.clone())).await;
    let dir = folder("changed");
    // The bytes on disk no longer hash to what was approved: the send is
    // cut before the closing boundary, and the error says why.
    let (request, _) = upload(
        format!("{base}/upload"),
        &dir,
        b"a,b\n1,2\n",
        Some("0".repeat(64)),
    );
    let error = network().fetch(request).await.unwrap_err();
    assert_eq!(error.public_code, "request_failed");
    assert_eq!(error.extra["reason"], "file_changed");
    assert!(itda_net::sent(&error));

    // The file shrank between the approval and the send: nothing leaves.
    let (request, path) = upload(format!("{base}/upload"), &dir, b"a,b\n1,2\n3,4\n", None);
    std::fs::write(&path, b"a,b\n").unwrap();
    let error = network().fetch(request).await.unwrap_err();
    assert_eq!(error.public_code, "data_invalid");
    assert_eq!(error.extra["reason"], "file_changed");
    assert!(!itda_net::sent(&error));
    assert!(received.lock().unwrap().body.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_file_that_changes_during_the_send_aborts_it() {
    // The upload reader's seam runs between two chunks, on the sending
    // side: the file is changed there, so the check before the second
    // chunk always sees it, whatever the host's socket buffers hold.
    let (outcome, seen) = tokio::sync::oneshot::channel::<String>();
    let outcome = Arc::new(Mutex::new(Some(outcome)));
    let router = Router::new().route(
        "/upload",
        post(move |body: axum::body::Body| {
            let outcome = outcome.clone();
            async move {
                use futures_util::StreamExt;
                let mut stream = body.into_data_stream();
                let mut total = 0usize;
                let mut result = "complete".to_owned();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => total += chunk.len(),
                        Err(_) => {
                            result = "cut".into();
                            break;
                        }
                    }
                }
                if let Some(outcome) = outcome.lock().unwrap().take() {
                    let _ = outcome.send(format!("{result} after {total}"));
                }
                StatusCode::CREATED
            }
        }),
    );
    let base = host(router).await;
    let dir = folder("mid-send");
    // Three chunks: the seam fires before the second and the third.
    let content = vec![b'z'; 200 * 1024];
    let (mut request, path) = upload(format!("{base}/upload"), &dir, &content, None);
    let changed_at = Arc::new(Mutex::new(None));
    let seam = {
        let (path, changed_at) = (path.clone(), changed_at.clone());
        Arc::new(move |sent: u64| {
            let mut changed_at = changed_at.lock().unwrap();
            if changed_at.is_none() {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap();
                file.write_all(b"one more line\n").unwrap();
                *changed_at = Some(sent);
            }
        })
    };
    request.multipart.as_mut().unwrap().between_chunks = Some(seam);
    let error = network().fetch(request).await.unwrap_err();
    assert_eq!(error.public_code, "request_failed", "{error:?}");
    assert_eq!(error.extra["reason"], "file_changed");
    assert!(itda_net::sent(&error));
    assert_eq!(*changed_at.lock().unwrap(), Some(64 * 1024));
    let seen = seen.await.unwrap();
    assert!(seen.starts_with("cut after "), "{seen}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_lease_is_renewed_while_held_and_lost_when_taken_away() {
    let store = Store::in_memory().unwrap();
    let gate = Gate::with_limits(store.clone(), 0, 1000).unwrap();
    let lease = gate.acquire("https://site.test|*", None).await.unwrap();
    lease.renew().await.unwrap();
    // Another process took the scope over: the turn is no longer this one's.
    store
        .call(|conn| {
            conn.execute("UPDATE request_gate SET lease='other'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let error = lease.renew().await.unwrap_err();
    assert_eq!(error.public_code, "request_lease_lost");
}
