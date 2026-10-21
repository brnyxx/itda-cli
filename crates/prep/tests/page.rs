#![allow(clippy::unwrap_used)]
use itda_prep::{Options, Page, from_pages};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/").to_owned() + name,
    )
    .unwrap()
}

fn page(url: &str, file: &str) -> Page {
    Page {
        url: url.into(),
        html: fixture(file),
    }
}

#[test]
fn a_board_page_becomes_a_list_a_search_a_post_and_a_feed() {
    let draft = from_pages(
        &[page(
            "https://intranet.example.co.kr/board?page=1",
            "board.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.site, "intranet-example");
    assert_eq!(draft.operations, 4, "{}", draft.yaml);
    assert_eq!(draft.reads, 3);
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.site.base_url, "https://intranet.example.co.kr");

    let board = spec.op("get-board").unwrap();
    let response = board.response.as_ref().unwrap();
    assert_eq!(response.format, "html");
    assert_eq!(response.required_html_fields[0].selector, "h1");
    let html = response.html.as_ref().unwrap();
    assert_eq!(html.items, "table tbody tr");
    assert_eq!(html.fields["title"].selector, "td:nth-child(2)");
    assert!(html.fields["link"].absolute_url);
    assert_eq!(board.query["page"].kind, "int");
    assert_eq!(board.query["page"].default, Some(1.into()));

    let search = spec.op("search-board").unwrap();
    assert_eq!(search.method, "GET");
    assert_eq!(search.path, "/board/search");
    assert!(search.query["q"].required);
    assert_eq!(search.query["board_id"].default, Some("7".into()));
    assert_eq!(search.query["sort"].kind, "string");

    let write = spec.op("post-write").unwrap();
    assert_eq!(write.auth, "session");
    let body = write.body.as_ref().unwrap();
    assert!(body.form["title"].required);
    assert!(body.form["_csrf"].from_step());
    assert_eq!(write.preflight[0].path, "/board?page=1");
    assert_eq!(
        write.preflight[0].extract["csrf"],
        "css:input[name=\"_csrf\"]@value"
    );

    let feed = spec.op("feed").unwrap();
    assert_eq!(feed.path, "/board/feed.xml");
    assert_eq!(feed.response.as_ref().unwrap().format, "xml");

    assert_eq!(spec.auth[0].kind, "browser");
    assert!(!draft.yaml.contains("recorded-secret-token"));
    assert!(!draft.yaml.contains("name: pw"));
    assert!(draft.notes.iter().any(|n| n.contains("sign-in form")));
}

#[test]
fn an_app_shell_reads_its_embedded_data_and_points_at_recording() {
    let draft = from_pages(
        &[page("https://shop.example/products", "app.html")],
        &Options {
            name: Some("shop".into()),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(draft.operations, 1);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let products = spec.op("list-products").unwrap();
    let response = products.response.as_ref().unwrap();
    assert_eq!(response.ssr_json, "__NEXT_DATA__");
    assert_eq!(response.required_pointers, ["/props"]);
    assert!(!draft.yaml.contains("private-value"));
    assert!(draft.notes.iter().any(|n| n.contains("__NEXT_DATA__")));
}

#[test]
fn pages_elsewhere_and_duplicates_are_left_out() {
    let draft = from_pages(
        &[
            page("https://intranet.example.co.kr/board", "board.html"),
            page("https://intranet.example.co.kr/board", "board.html"),
            page("https://other.example/board", "board.html"),
        ],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 4);
    assert!(draft.notes.iter().any(|n| n.contains("not under")));
    assert!(draft.notes.iter().any(|n| n.contains("drafted once")));
    assert!(from_pages(&[], &Options::default()).is_err());
    assert!(from_pages(&[page("ftp://x/", "app.html")], &Options::default()).is_err());
}
