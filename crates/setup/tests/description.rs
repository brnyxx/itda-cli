//! Finding the API description a site points at, against a local fixture:
//! the first page's `Link` header, an API catalog (a linkset), and a
//! `describedby` that may or may not be a description.
#![allow(clippy::unwrap_used)]
use axum::{Router, extract::Request, routing::get};
use itda_net::{Gate, Network};
use itda_prep::Page;
use itda_store::Store;
use std::sync::{Arc, Mutex};

const OPENAPI: &str =
    r#"{"openapi": "3.0.3", "info": {"title": "Board", "version": "1"}, "paths": {}}"#;

async fn serve(hits: Arc<Mutex<Vec<String>>>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let catalog = format!(
        r#"{{"linkset": [{{"anchor": "{base}/apis/board", "service-doc": [{{"href": "/apis/board/doc"}}], "service-desc": [{{"href": "/apis/board/spec", "type": "application/json"}}]}}]}}"#
    );
    let json = |body: String| ([("content-type", "application/json")], body);
    let router = Router::new()
        .route("/spec/v1.json", get(move || async move { json(OPENAPI.into()) }))
        .route("/apis/board/spec", get(move || async move { json(OPENAPI.into()) }))
        .route("/openapi.json", get(move || async move { json(OPENAPI.into()) }))
        .route(
            "/.well-known/api-catalog",
            get(move || async move {
                (
                    [(
                        "content-type",
                        r#"application/linkset+json; profile="https://www.rfc-editor.org/info/rfc9727""#,
                    )],
                    catalog,
                )
            }),
        )
        .route(
            "/about.html",
            get(|| async { axum::response::Html("<title>About</title>") }),
        )
        .layer(axum::middleware::from_fn(
            move |request: Request, next: axum::middleware::Next| {
                hits.lock().unwrap().push(request.uri().path().to_owned());
                next.run(request)
            },
        ));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn net() -> Network {
    let store = Store::in_memory().unwrap();
    Network::new(Arc::new(Gate::with_limits(store, 0, 1000).unwrap()), true)
}

fn page(base: &str, html: &str, link: &str) -> Page {
    Page {
        url: format!("{base}/board"),
        html: format!("<title>Board</title>{html}"),
        link: link.into(),
    }
}

async fn found(page: Page) -> (Option<String>, Vec<String>) {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let base = serve(hits.clone()).await;
    let page = Page {
        url: page.url.replace("BASE", &base),
        html: page.html.replace("BASE", &base),
        link: page.link.replace("BASE", &base),
    };
    let description = itda_setup::description::find(&net(), &[page]).await;
    let path = description.map(|d| url::Url::parse(&d.url).unwrap().path().to_owned());
    let hits = hits.lock().unwrap().clone();
    (path, hits)
}

#[tokio::test]
async fn the_link_header_names_the_description_and_learning_it_costs_nothing() {
    let (path, hits) = found(page("BASE", "", r#"</spec/v1.json>; rel="service-desc""#)).await;
    assert_eq!(path.as_deref(), Some("/spec/v1.json"));
    assert_eq!(hits, ["/spec/v1.json"]);

    // A header outranks a weaker link in the page.
    let (path, hits) = found(page(
        "BASE",
        r#"<link rel="describedby" href="/about.html">"#,
        r#"<BASE/spec/v1.json>; rel="nofollow Service-Desc""#,
    ))
    .await;
    assert_eq!(path.as_deref(), Some("/spec/v1.json"));
    assert_eq!(hits, ["/spec/v1.json"]);
}

#[tokio::test]
async fn an_api_catalog_leads_to_its_service_desc() {
    let (path, hits) = found(page(
        "BASE",
        "",
        r#"</.well-known/api-catalog>; rel="api-catalog""#,
    ))
    .await;
    assert_eq!(path.as_deref(), Some("/apis/board/spec"));
    assert_eq!(hits, ["/.well-known/api-catalog", "/apis/board/spec"]);

    let (path, _) = found(page(
        "BASE",
        r#"<a rel="api-catalog" href="/.well-known/api-catalog">Our APIs</a>"#,
        "",
    ))
    .await;
    assert_eq!(path.as_deref(), Some("/apis/board/spec"));
}

#[tokio::test]
async fn describedby_counts_only_when_it_is_a_description() {
    let (path, hits) = found(page(
        "BASE",
        r#"<link rel="describedby" href="/about.html">"#,
        "",
    ))
    .await;
    assert_eq!(path.as_deref(), Some("/openapi.json"));
    assert_eq!(hits, ["/about.html", "/openapi.json"]);

    let (path, hits) = found(page("BASE", "", r#"</spec/v1.json>; rel=describedby"#)).await;
    assert_eq!(path.as_deref(), Some("/spec/v1.json"));
    assert_eq!(hits, ["/spec/v1.json"]);
}
