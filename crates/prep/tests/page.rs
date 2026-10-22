#![allow(clippy::unwrap_used)]
use itda_prep::{Described, Description, Options, Page, from_pages, from_pages_and_description};

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
    assert_eq!(response.extract_pointer, "/props/pageProps/products");
    assert_eq!(response.required_pointers, ["/0/id", "/0/name"]);
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

fn described(server: &str, extra_path: &str) -> String {
    let servers = if server.is_empty() {
        String::new()
    } else {
        format!(r#""servers": [{{"url": "{server}"}}],"#)
    };
    format!(
        r#"{{"openapi": "3.0.3", "info": {{"title": "Pet API", "version": "2.0"}}, {servers}
        "components": {{"securitySchemes": {{"token": {{"type": "http", "scheme": "bearer"}}}}}},
        "paths": {{
          "/pets": {{
            "get": {{"operationId": "listPets", "summary": "List pets",
                     "responses": {{"200": {{"content": {{"application/json": {{"schema": {{"type": "object", "required": ["items"]}}}}}}}}}}}},
            "post": {{"operationId": "createPet", "security": [{{"token": []}}],
                      "requestBody": {{"content": {{"application/json": {{"schema": {{"type": "object", "properties": {{"name": {{"type": "string"}}}}}}}}}}}},
                      "responses": {{"201": {{"description": "made"}}}}}}
          }},
          "/pets/{{petId}}": {{
            "get": {{"operationId": "showPet", "security": [{{"token": []}}],
                     "parameters": [{{"name": "petId", "in": "path", "required": true, "schema": {{"type": "integer"}}}}],
                     "responses": {{"200": {{"content": {{"application/json": {{"schema": {{"type": "object", "required": ["id"]}}}}}}}}}}}},
            "delete": {{"operationId": "removePet", "responses": {{"204": {{"description": "gone"}}}}}}
          }},
          "{extra_path}": {{
            "get": {{"operationId": "extra", "responses": {{"200": {{"description": "ok"}}}}}}
          }},
          "/secure": {{
            "get": {{"operationId": "secure",
                     "parameters": [{{"name": "apikey", "in": "query", "schema": {{"type": "string"}}}}],
                     "responses": {{"200": {{"description": "ok"}}}}}}
          }}
        }}}}"#
    )
}

#[test]
fn a_linked_description_adds_its_reads_and_notes_its_changes() {
    let page = page("https://pets.example/docs", "docs.html");
    let description = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: described("/api/v2", "/docs").into_bytes(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&description),
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(
        draft.described,
        Some(Described {
            url: "https://pets.example/openapi.json".into(),
            reads: 3,
            changes: 2,
        })
    );
    assert_eq!((draft.operations, draft.reads), (4, 4), "{}", draft.yaml);
    assert!(
        draft
            .yaml
            .starts_with("# Drafted by `itda draft page` from pages on pets.example and the API description at https://pets.example/openapi.json."),
        "{}",
        draft.yaml
    );
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.site.base_url, "https://pets.example");
    assert_eq!(spec.site.name, "pets");

    let list = spec.op("list-pets").unwrap();
    assert_eq!(
        list.path, "/api/v2/pets",
        "a relative server is a path prefix"
    );
    assert_eq!(list.desc, "List pets");
    assert_eq!(
        list.evidence,
        "OpenAPI Pet API 2.0 at https://pets.example/openapi.json: GET /pets"
    );
    assert_eq!(
        list.response.as_ref().unwrap().required_pointers,
        ["/items"]
    );
    let show = spec.op("show-pet").unwrap();
    assert_eq!(show.auth, "token");
    assert_eq!(show.params["petId"].kind, "int");
    assert_eq!(spec.op("extra").unwrap().path, "/api/v2/docs");
    assert_eq!(
        spec.auth[0].value_from, "store:pets/token",
        "auth belongs to the page's site, not the document's title"
    );

    assert!(spec.op("create-pet").is_err());
    assert!(spec.op("remove-pet").is_err());
    assert!(
        spec.op("secure").is_err(),
        "a read keyed by an input is not drafted: {}",
        draft.yaml
    );
    assert!(
        draft.notes.iter().any(|n| n
            == "GET /secure: left out; its input apikey is a credential, which goes in auth, never through the agent."),
        "{:?}",
        draft.notes
    );
    assert!(
        draft.notes.iter().any(|n| n
            .contains("`itda draft openapi https://pets.example/openapi.json` drafts the changes")),
        "{:?}",
        draft.notes
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.starts_with("POST /pets: a change the API description lists")),
        "{:?}",
        draft.notes
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.starts_with("DELETE /pets/{petId}: a change")),
        "{:?}",
        draft.notes
    );
    assert!(
        !draft
            .notes
            .iter()
            .any(|n| n.contains("`itda draft openapi` makes a fuller")),
        "the old hint goes once the description is read: {:?}",
        draft.notes
    );
}

#[test]
fn a_description_without_servers_lives_where_it_was_found_and_dedupes_by_path() {
    let page = page("https://pets.example/docs", "docs.html");
    let description = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: described("", "/docs").into_bytes(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&description),
        &Options::default(),
    )
    .unwrap();
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.op("list-pets").unwrap().path, "/pets");
    assert!(
        spec.op("extra").is_err(),
        "GET /docs is the page itself: {}",
        draft.yaml
    );
    assert_eq!(
        spec.op("list-docs")
            .unwrap()
            .response
            .as_ref()
            .unwrap()
            .format,
        "html"
    );
    assert_eq!(draft.described.as_ref().unwrap().reads, 2);
}

#[test]
fn a_description_elsewhere_or_unreadable_leaves_the_page_draft_alone() {
    let page = page("https://pets.example/docs", "docs.html");
    let plain = from_pages(std::slice::from_ref(&page), &Options::default()).unwrap();
    assert_eq!(plain.operations, 1);
    assert!(plain.described.is_none());
    assert!(
        plain.notes.iter().any(|n| n.contains(
            "links to an API description at https://pets.example/openapi.json; `itda draft openapi`"
        )),
        "{:?}",
        plain.notes
    );

    let elsewhere = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: described("https://api.pets.example/v2", "/x").into_bytes(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&elsewhere),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1);
    assert!(draft.described.is_none());
    assert!(
        draft.notes.iter().any(|n| n.contains(
            "describes an API at https://api.pets.example/v2, not under https://pets.example"
        )),
        "{:?}",
        draft.notes
    );

    let broken = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: b"<html>not here</html>".to_vec(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&broken),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1);
    assert!(draft.described.is_none());
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("could not be read as an API description")),
        "{:?}",
        draft.notes
    );
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
}
