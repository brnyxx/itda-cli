#![allow(clippy::unwrap_used)]
use itda_policy::{Effect, operation_effect};
use itda_prep::{Options, from_openapi};
use std::collections::BTreeMap;

fn draft(file: &str, options: Options) -> itda_prep::Draft {
    let text = std::fs::read(format!(
        "{}/tests/fixtures/{file}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    from_openapi(&text, &options).unwrap()
}

#[test]
fn openapi3_becomes_a_valid_reviewed_draft() {
    let draft = draft("petstore.yaml", Options::default());
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(draft.site, "pet-store");
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.site.base_url, "https://api.pets.example/v1");
    assert!(draft.yaml.starts_with("# Drafted by `itda draft openapi`"));

    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "list-owners",
            "list-pets",
            "create-pet",
            "show-pet-by-id",
            "delete-pet"
        ]
    );
    assert_eq!((draft.operations, draft.reads), (5, 3));

    let list = spec.op("list-pets").unwrap();
    assert_eq!(list.desc, "List pets, newest first");
    assert!(list.auth.is_empty(), "security: [] means signed out works");
    assert_eq!(list.query["limit"].kind, "int");
    assert_eq!(list.query["limit"].default, Some(serde_json::json!(20)));
    let response = list.response.as_ref().unwrap();
    assert_eq!(response.required_pointers, ["/items", "/total"]);
    assert!(list.evidence.contains("Pet Store 1.2.0: GET /pets"));

    let show = spec.op("show-pet-by-id").unwrap();
    assert_eq!(show.auth, "api-key");
    assert_eq!(show.params["petId"].kind, "int");

    let create = spec.op("create-pet").unwrap();
    let body = create.body.as_ref().unwrap();
    assert!(body.json.contains_key("name") && body.json["name"].required);
    assert!(
        !body.json.contains_key("id"),
        "read-only fields are not inputs"
    );
    assert_eq!(operation_effect(create, &BTreeMap::new()), Effect::Write);
    // GET /pets declares /items but not what a pet looks like: no confirm.
    assert!(create.confirm.is_none(), "{}", draft.yaml);

    let delete = spec.op("delete-pet").unwrap();
    assert!(delete.auth.is_empty(), "oauth is not supported yet");
    assert_eq!(delete.response.as_ref().unwrap().format, "empty");

    let auth: Vec<&str> = spec.auth.iter().map(|a| a.value_from.as_str()).collect();
    assert_eq!(auth, ["store:pet-store/api-key"]);
}

#[test]
fn what_cannot_be_translated_is_listed() {
    let notes = draft("petstore.yaml", Options::default()).notes.join("\n");
    assert!(notes.contains("oauth"), "{notes}");
    assert!(notes.contains("header parameter X-Trace"), "{notes}");
    assert!(
        notes.contains("PUT /pets/{petId}/photo: skipped"),
        "{notes}"
    );
    assert!(
        notes.contains("GET /legacy: skipped because it is deprecated"),
        "{notes}"
    );
}

#[test]
fn swagger2_and_options() {
    let draft = draft(
        "swagger2.json",
        Options {
            name: Some("Board".into()),
            include: Some("notices".into()),
            ..Options::default()
        },
    );
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.site.name, "board");
    assert_eq!(spec.site.base_url, "https://intranet.example/api");
    let search = spec.op("search-notices").unwrap();
    assert!(search.query["q"].required);
    let post = spec.op("post-notice").unwrap();
    assert_eq!(post.auth, "token");
    assert!(post.body.as_ref().unwrap().form["title"].required);

    let narrowed = from_openapi(
        &std::fs::read(format!(
            "{}/tests/fixtures/petstore.yaml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
        &Options {
            include: Some("admin".into()),
            base_url: Some("http://127.0.0.1:9000/".into()),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(narrowed.operations, 1);
    assert!(narrowed.yaml.contains("base_url: http://127.0.0.1:9000\n"));
}

#[test]
fn other_documents_are_refused() {
    assert!(from_openapi(b"{\"hello\": 1}", &Options::default()).is_err());
    assert!(from_openapi(b"not: [valid", &Options::default()).is_err());
    let relative = br#"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"servers":[{"url":"/v1"}],"paths":{}}"#;
    assert!(from_openapi(relative, &Options::default()).is_err());
}

#[test]
fn documented_page_turning_is_declared_one_way_per_style() {
    let pets = draft("petstore.yaml", Options::default());
    assert!(pets.problems.is_empty(), "{:?}", pets.problems);
    let spec = itda_spec::parse(pets.yaml.as_bytes()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();

    // A cursor parameter and a next_cursor field in the answer.
    let list = spec.op("list-pets").unwrap();
    let pagination = list.pagination.as_ref().unwrap();
    assert_eq!(pagination.style, "cursor");
    assert_eq!(pagination.next, "/meta/next_cursor");
    assert_eq!(pagination.input(), ("cursor", false));
    assert_eq!(list.query["cursor"].kind, "string");

    // A page parameter and a documented Link header.
    let owners = spec.op("list-owners").unwrap();
    let pagination = owners.pagination.as_ref().unwrap();
    assert_eq!(
        (pagination.style.as_str(), pagination.next.as_str()),
        ("page", "header:link")
    );
    assert_eq!(pagination.input(), ("page", false));

    // Nothing documented, nothing declared.
    assert!(spec.op("show-pet-by-id").unwrap().pagination.is_none());
    assert!(spec.op("create-pet").unwrap().pagination.is_none());

    // An offset parameter.
    let board = draft("swagger2.json", Options::default());
    let spec = itda_spec::parse(board.yaml.as_bytes()).unwrap();
    let search = spec.op("search-notices").unwrap();
    let pagination = search.pagination.as_ref().unwrap();
    assert_eq!(
        (pagination.style.as_str(), pagination.next.as_str()),
        ("offset", "")
    );
    assert_eq!(pagination.input(), ("offset", false));
    assert!(
        board.yaml.contains("style: offset") && board.yaml.contains("query: offset"),
        "{}",
        board.yaml
    );
}
