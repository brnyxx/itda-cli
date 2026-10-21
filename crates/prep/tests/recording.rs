#![allow(clippy::unwrap_used)]
use itda_prep::{Options, from_recording};
use serde_json::json;

fn board_page() -> &'static str {
    "<title>Board</title><h1>Notices</h1><table><thead><tr><th>No</th><th>Title</th></tr></thead>
     <tr><td>2</td><td><a href='/n/2'>Second</a></td></tr><tr><td>1</td><td><a href='/n/1'>First</a></td></tr></table>
     <form action='/board/search' method='get'><input name='q'></form>"
}

fn entry(method: &str, url: &str, mime: &str, text: &str) -> serde_json::Value {
    json!({"request": {"method": method, "url": url, "headers": []},
           "response": {"status": 200, "content": {"mimeType": mime, "text": text}}})
}

#[test]
fn pages_in_a_recording_become_actions_next_to_the_calls() {
    let har = json!({"log": {"entries": [
        entry("GET", "https://intranet.example/board", "text/html; charset=utf-8", board_page()),
        entry("GET", "https://intranet.example/api/items?q=x", "application/json", "{\"items\": []}"),
        entry("GET", "https://intranet.example/board#top", "text/html", board_page()),
    ]}});
    let draft = from_recording(
        &serde_json::to_vec(&har).unwrap(),
        &Options::default(),
        "`itda record`",
    )
    .unwrap();
    assert!(
        draft
            .yaml
            .starts_with("# Drafted by `itda record` from a recording of intranet.example"),
        "{}",
        draft.yaml
    );
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["list-items", "get-board", "search-board"]);
    let board = spec.op("get-board").unwrap();
    assert_eq!(
        board
            .response
            .as_ref()
            .unwrap()
            .html
            .as_ref()
            .unwrap()
            .items,
        "table tbody tr"
    );
}

#[test]
fn a_recording_of_pages_alone_still_drafts_them() {
    let har = json!({"log": {"entries": [
        entry("GET", "https://intranet.example/board", "text/html", board_page()),
        entry("GET", "https://intranet.example/logo.png", "image/png", ""),
    ]}});
    let draft = from_recording(
        &serde_json::to_vec(&har).unwrap(),
        &Options::default(),
        "`itda record`",
    )
    .unwrap();
    assert_eq!(draft.site, "intranet");
    assert_eq!(draft.operations, 2);
    assert!(itda_prep::from_har(&serde_json::to_vec(&har).unwrap(), &Options::default()).is_err());

    let empty =
        json!({"log": {"entries": [entry("GET", "https://x.example/a.png", "image/png", "")]}});
    assert!(
        from_recording(
            &serde_json::to_vec(&empty).unwrap(),
            &Options::default(),
            "x"
        )
        .is_err()
    );
}
