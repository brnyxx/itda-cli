use axum::{
    Router,
    body::Bytes,
    http::{HeaderMap, Method},
    response::Html,
    routing::{any, get, post},
};
use itda_browser::transport::{BrowserMode, BrowserRequest, BrowserTransport};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[tokio::test(flavor = "multi_thread")]
async fn page_frame_and_worker_queries_run_but_mutations_never_reach_server() {
    if std::env::var_os("ITDA_TEST_BROWSER").is_none_or(|b| b.is_empty()) {
        eprintln!("skipped: set ITDA_TEST_BROWSER to run");
        return;
    }
    let queries = Arc::new(Mutex::new(Vec::new()));
    let seen = queries.clone();
    let foreign_seen = queries.clone();
    let foreign_log = Arc::new(Mutex::new(Vec::new()));
    let log = foreign_log.clone();
    let foreign = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = format!("http://{}/graphql", foreign.local_addr().unwrap());
    let foreign_server = tokio::spawn(async move {
        axum::serve(
            foreign,
            Router::new().route(
                "/graphql",
                any(move |method: Method, headers: HeaderMap, body: Bytes| {
                    let seen = foreign_seen.clone();
                    log.lock().unwrap().push((method.to_string(), headers));
                    async move {
                        if method == Method::POST {
                            let document: serde_json::Value =
                                serde_json::from_slice(&body).unwrap();
                            seen.lock()
                                .unwrap()
                                .push(document["query"].as_str().unwrap().to_owned());
                        } else {
                            assert_eq!(method, Method::OPTIONS);
                        }
                        (
                            [
                                ("access-control-allow-origin", "*"),
                                ("access-control-allow-methods", "POST"),
                                ("access-control-allow-headers", "content-type"),
                            ],
                            axum::Json(serde_json::json!({"data":{}})),
                        )
                    }
                }),
            ),
        )
        .await
        .unwrap()
    });
    let query_script = |name| {
        format!(
            "fetch('/graphql',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify({{query:'query {{ {name} }}'}})}}).then(r=>r.json()).then(()=>{{NOTIFY}},e=>{{FAIL}});fetch('/graphql',{{method:'POST',body:JSON.stringify({{query:'mutation {{ removeUser }}'}})}}).catch(()=>{{}});"
        )
    };
    // A query that fails says so in the page, so the assertion below shows
    // which one and why instead of a page still checking the browser.
    let page_query = query_script("page")
        .replace("NOTIFY", "ready('page')")
        .replace("FAIL", "ready('page failed: '+e)");
    let frame_query = query_script("frame")
        .replace("NOTIFY", "parent.postMessage('frame','*')")
        .replace("FAIL", "parent.postMessage('frame failed: '+e,'*')");
    let worker_query = query_script("worker")
        .replace("NOTIFY", "postMessage('worker')")
        .replace("FAIL", "postMessage('worker failed: '+e)")
        .replace("'/graphql'", &format!("'{remote}'"));
    let page=format!("<html><body><h1>Checking your browser</h1><script>
        const done=new Set();function ready(v){{if(String(v).includes('failed')){{document.querySelector('h1').textContent=v;return}}done.add(v);if(done.size===3)document.querySelector('h1').textContent='graphql ready'}}
        addEventListener('message',e=>ready(e.data));const w=new Worker('/worker.js');w.onmessage=e=>ready(e.data);
        {page_query}</script><iframe src='/frame'></iframe></body></html>");
    let router = Router::new()
        .route(
            "/page",
            get(move || {
                let page = page.clone();
                async { Html(page) }
            }),
        )
        .route(
            "/frame",
            get(move || {
                let frame = frame_query.clone();
                async move { Html(format!("<script>{frame}</script>")) }
            }),
        )
        .route(
            "/worker.js",
            get(move || {
                let worker = worker_query.clone();
                async { ([("content-type", "text/javascript")], worker) }
            }),
        )
        .route(
            "/graphql",
            post(move |body: Bytes| {
                let seen = seen.clone();
                async move {
                    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    seen.lock()
                        .unwrap()
                        .push(document["query"].as_str().unwrap().to_owned());
                    axum::Json(serde_json::json!({"data":{}}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let response = tokio::task::spawn_blocking(move || {
        BrowserTransport::new()
            .execute(BrowserRequest {
                url: format!("{origin}/page"),
                method: "GET".into(),
                headers: Vec::new(),
                body: None,
                account_id: "graphql".into(),
                credential_fingerprint: Some(String::new()),
                read_only: true,
                limit: 128 * 1024,
                timeout: Duration::from_secs(15),
                allow_local: true,
                mode: BrowserMode::Render,
                cooldowns: Vec::new(),
            })
            .unwrap()
    })
    .await
    .unwrap();
    let html = String::from_utf8(response.bytes).unwrap();
    assert!(
        html.contains("<h1>graphql ready</h1>"),
        "{html}; queries: {:?}; foreign: {:?}",
        queries.lock().unwrap(),
        foreign_log.lock().unwrap()
    );
    let mut queries = queries.lock().unwrap().clone();
    queries.sort();
    assert_eq!(
        queries,
        ["query { frame }", "query { page }", "query { worker }"]
    );
    server.abort();
    foreign_server.abort();
}
