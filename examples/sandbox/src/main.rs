//! A legacy-style intranet notice board for trying itda without touching a
//! real website. It serves server-rendered HTML, a small JSON API behind an
//! API key, and a page that refuses automated access, so every path of the
//! read pipeline can be seen locally.
//!
//! `cargo run -p itda-sandbox [-- PORT]` listens on 127.0.0.1 (default 8787).
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// The API key the sandbox accepts. It is public on purpose.
const API_KEY: &str = "sandbox-key";

#[derive(Clone)]
struct Board {
    notices: Arc<Mutex<Vec<Value>>>,
}

impl Board {
    fn notices(&self) -> Vec<Value> {
        self.notices.lock().map(|n| n.clone()).unwrap_or_default()
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);
    let board = Board {
        notices: Arc::new(Mutex::new(vec![
            notice(1, "10월 정기 점검 안내", "운영팀", "2026-10-01"),
            notice(2, "보안 교육 이수 요청", "보안팀", "2026-10-07"),
            notice(3, "사내 메신저 업데이트", "IT지원", "2026-10-15"),
        ])),
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!("sandbox listening on http://127.0.0.1:{port}");
    axum::serve(listener, router(board)).await
}

fn notice(id: u64, title: &str, author: &str, date: &str) -> Value {
    json!({"id": id, "title": title, "author": author, "date": date})
}

fn router(board: Board) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/notices", get(notice_page))
        .route("/api/notices", get(list_notices).post(create_notice))
        .route("/api/me", get(me))
        .route("/reports", get(reports))
        .with_state(board)
}

async fn home() -> Html<&'static str> {
    Html(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Sandbox intranet</title></head>
<body><h1>Sandbox intranet</h1><ul>
<li><a href=\"/notices\">Notices</a></li><li><a href=\"/reports\">Reports</a></li>
</ul></body></html>",
    )
}

/// Server-rendered table, the way many older internal systems publish data.
async fn notice_page(State(board): State<Board>) -> Html<String> {
    let rows: String = board
        .notices()
        .iter()
        .map(|n| {
            format!(
                "<tr class=\"notice\"><td class=\"id\">{}</td><td class=\"title\"><a href=\"/notices/{}\">{}</a></td><td class=\"author\">{}</td><td class=\"date\">{}</td></tr>",
                n["id"],
                n["id"],
                text(&n["title"]),
                text(&n["author"]),
                text(&n["date"])
            )
        })
        .collect();
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Notices</title></head>
<body><h1 id=\"board\">Notices</h1><table><tbody>{rows}</tbody></table></body></html>"
    ))
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn list_notices(
    State(board): State<Board>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    let needle = query.get("q").map(String::as_str).unwrap_or_default();
    let items: Vec<Value> = board
        .notices()
        .into_iter()
        .filter(|n| n["title"].as_str().unwrap_or_default().contains(needle))
        .collect();
    Json(json!({"total": items.len(), "items": items}))
}

fn signed_in(headers: &HeaderMap) -> bool {
    headers
        .get("x-api-key")
        .is_some_and(|value| value.as_bytes() == API_KEY.as_bytes())
}

async fn me(headers: HeaderMap) -> impl IntoResponse {
    if !signed_in(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(json!({"name": "Sandbox User", "team": "운영팀"})))
}

/// A change. itda refuses to send it until approvals exist.
async fn create_notice(
    State(board): State<Board>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if !signed_in(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut notices = board
        .notices
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let created = notice(
        notices.len() as u64 + 1,
        body["title"].as_str().unwrap_or("untitled"),
        "Sandbox User",
        "2026-10-19",
    );
    notices.push(created.clone());
    Ok((StatusCode::CREATED, Json(created)))
}

/// A page that refuses automated access. itda pauses the site instead of
/// retrying until the user runs `itda resume sandbox`.
async fn reports() -> impl IntoResponse {
    (
        StatusCode::FORBIDDEN,
        Html("<!doctype html><html><body><h1>Access denied</h1></body></html>"),
    )
}
