//! Ask the GraphQL endpoints a draft saw what they offer: one introspection
//! query each, sent as itda, read-only and bounded. An endpoint that refuses,
//! errors or answers something else is skipped without a word; sites turn
//! introspection off on purpose.
use itda_net::{Network, Request};
use itda_prep::graphql::{ENDPOINT_CAP, Schema, introspection_body};

const LIMIT: usize = 8 * 1024 * 1024;

/// The schemas of the endpoints that answered, at most `ENDPOINT_CAP` of them.
pub async fn schemas(net: &Network, endpoints: &[String]) -> Vec<Schema> {
    let mut out = Vec::new();
    for url in endpoints.iter().take(ENDPOINT_CAP) {
        let request = Request {
            method: "POST".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: Some(introspection_body()),
            limit: LIMIT,
            ..Request::read(url.as_str())
        };
        let Ok(response) = net.fetch(request).await else {
            continue;
        };
        if !response.successful() {
            continue;
        }
        let Some(json) = response.json() else {
            continue;
        };
        if !json["data"]["__schema"].is_object() {
            continue;
        }
        out.push(Schema {
            url: url.clone(),
            json,
        });
    }
    out
}

/// The endpoints the pages name, each once.
pub fn endpoints_in_pages(pages: &[itda_prep::Page]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for url in pages.iter().flat_map(itda_prep::graphql::endpoints_in_page) {
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}
