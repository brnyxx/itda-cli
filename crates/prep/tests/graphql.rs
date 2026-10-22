#![allow(clippy::unwrap_used)]
use itda_prep::graphql::{
    FIELD_CAP, INTROSPECTION_QUERY, Schema, endpoints_in_page, endpoints_in_recording,
};
use itda_prep::{Options, Page, from_har, from_pages};
use itda_spec::Effect;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn schema_json() -> Value {
    serde_json::from_str(include_str!("fixtures/graphql_schema.json")).unwrap()
}

fn board() -> Page {
    Page {
        url: "https://app.example/board".into(),
        html: "<title>Board</title><h1>Notices</h1><script>fetch('/graphql', {method: 'POST'})</script>"
            .into(),
    }
}

fn options(schemas: Vec<Schema>) -> Options {
    Options {
        schemas,
        ..Options::default()
    }
}

#[test]
fn a_schema_becomes_one_read_per_query_field_that_takes_plain_inputs() {
    let endpoints = endpoints_in_page(&board());
    assert_eq!(endpoints, ["https://app.example/graphql"]);
    let draft = from_pages(
        &[board()],
        &options(vec![Schema {
            url: endpoints[0].clone(),
            json: schema_json(),
        }]),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();
    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "get-board",
            "notices",
            "notice",
            "search-notices",
            "notices-by-ids",
            "me",
            "version",
            "wide",
            "search"
        ]
    );
    assert_eq!((draft.operations, draft.reads), (9, 9));

    let notices = spec.op("notices").unwrap();
    assert_eq!(notices.desc, "Query notices");
    assert_eq!(
        (notices.method.as_str(), notices.path.as_str()),
        ("POST", "/graphql")
    );
    assert_eq!(
        itda_policy::operation_effect(notices, &BTreeMap::new()),
        Effect::Read
    );
    let body = notices.body.as_ref().unwrap();
    assert_eq!(
        body.json["query"]
            .default
            .as_ref()
            .unwrap()
            .as_str()
            .unwrap(),
        "query Notices($first: Int, $status: Status, $pinned: Boolean) { notices(first: $first, status: $status, pinned: $pinned) { id title status author { id name } tags { label weight } } }"
    );
    assert_eq!(body.json["operationName"].default, Some("Notices".into()));
    assert_eq!(body.json["first"].kind, "int");
    assert!(
        !body.json["first"].required,
        "the schema has a default for it"
    );
    assert_eq!(body.json["status"].kind, "string");
    assert_eq!(body.json["pinned"].kind, "bool");
    let template = body.json_template.as_ref().unwrap();
    assert_eq!(template["variables"]["pinned"], "{{pinned}}");
    assert_eq!(
        notices.response.as_ref().unwrap().required_pointers,
        ["/data/notices"]
    );
    assert!(notices.evidence.contains("lists notices under Query"));

    // An input the agent leaves out leaves the body; one it gives is typed.
    let (_, sent) = spec.build_request(notices, &BTreeMap::new()).unwrap();
    let sent: Value = serde_json::from_slice(&sent.unwrap()).unwrap();
    assert_eq!(sent["operationName"], "Notices");
    assert_eq!(sent["variables"], json!({}));
    let mut args = BTreeMap::new();
    args.insert("first".to_owned(), "5".to_owned());
    let (_, sent) = spec.build_request(notices, &args).unwrap();
    let sent: Value = serde_json::from_slice(&sent.unwrap()).unwrap();
    assert_eq!(sent["variables"], json!({"first": 5}));

    let notice = spec.op("notice").unwrap();
    assert!(notice.body.as_ref().unwrap().json["id"].required);
    assert!(
        notice.body.as_ref().unwrap().json["query"]
            .default
            .as_ref()
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("query Notice($id: ID!) { notice(id: $id) {")
    );

    let me = spec.op("me").unwrap();
    let me_body = me.body.as_ref().unwrap();
    assert!(
        !me_body.json.contains_key("sessionId"),
        "a credential is never an input"
    );
    assert_eq!(
        me_body.json["query"].default,
        Some("query Me { me { id name team { name } } }".into())
    );

    // An argument named like the document's own inputs gets the field's name
    // in front, so the document stays fixed and the text stays an input.
    let search = spec.op("search").unwrap();
    let search_body = search.body.as_ref().unwrap();
    assert!(search_body.json["search_query"].required);
    assert!(
        search_body.json["query"]
            .default
            .as_ref()
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with(
                "query Search($search_query: String!, $first: Int) { search(query: $search_query, first: $first) {"
            ),
        "{:?}",
        search_body.json["query"].default
    );
    let mut args = BTreeMap::new();
    args.insert("search_query".to_owned(), "hello".to_owned());
    let (_, sent) = spec.build_request(search, &args).unwrap();
    let sent: Value = serde_json::from_slice(&sent.unwrap()).unwrap();
    assert_eq!(sent["variables"], json!({"search_query": "hello"}));
    assert!(sent["query"].as_str().unwrap().starts_with("query Search("));

    let version = spec.op("version").unwrap();
    assert_eq!(
        version.body.as_ref().unwrap().json["query"].default,
        Some("query Version { version }".into())
    );
    for op in &spec.operations {
        assert_eq!(
            itda_policy::operation_effect(op, &BTreeMap::new()),
            Effect::Read,
            "{}",
            op.name
        );
    }

    let notes = draft.notes.join("\n");
    assert!(
        notes.contains(
            "drafted searchNotices (filter), noticesByIds (ids) without the argument in brackets"
        ),
        "{notes}"
    );
    assert_eq!(
        spec.op("search-notices").unwrap().body.as_ref().unwrap().json["query"].default,
        Some("query SearchNotices { searchNotices { id title status author { id name } tags { label weight } } }".into())
    );
    assert!(
        notes.contains("left out viewer; they take a credential"),
        "{notes}"
    );
    assert!(
        notes.contains("left out purgeCache; the name says it changes something"),
        "{notes}"
    );
    assert!(notes.contains("left out empty; no plain field"), "{notes}");
    assert!(
        notes.contains("Mutation lists 2 changes (createNotice, deleteNotice)"),
        "{notes}"
    );
    assert!(
        !draft.yaml.contains("createNotice"),
        "changes are never drafted"
    );
    assert!(
        !draft.yaml.contains("oldNotices"),
        "deprecated fields are skipped"
    );
}

#[test]
fn a_selection_stops_at_the_field_cap() {
    let draft = from_pages(
        &[board()],
        &options(vec![Schema {
            url: "https://app.example/graphql".into(),
            json: schema_json(),
        }]),
    )
    .unwrap();
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let wide = spec.op("wide").unwrap();
    let document = wide.body.as_ref().unwrap().json["query"]
        .default
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let selected = document
        .split_once("{ wide { ")
        .unwrap()
        .1
        .trim_end_matches([' ', '}']);
    assert_eq!(selected.split(' ').count(), FIELD_CAP, "{document}");
    assert!(
        !document.contains("more"),
        "no room was left for it: {document}"
    );
}

#[test]
fn a_disabled_or_foreign_schema_adds_nothing() {
    let disabled = from_pages(
        &[board()],
        &options(vec![Schema {
            url: "https://app.example/graphql".into(),
            json: json!({"errors": [{"message": "introspection is disabled"}]}),
        }]),
    )
    .unwrap();
    assert_eq!(disabled.operations, 1);
    assert!(disabled.problems.is_empty(), "{:?}", disabled.problems);
    assert!(
        !disabled.notes.iter().any(|n| n.contains("graphql")),
        "{:?}",
        disabled.notes
    );

    let elsewhere = from_pages(
        &[board()],
        &options(vec![Schema {
            url: "https://api.other.example/graphql".into(),
            json: schema_json(),
        }]),
    )
    .unwrap();
    assert_eq!(elsewhere.operations, 1);
    assert!(
        elsewhere
            .notes
            .iter()
            .any(|n| n.contains("not under https://app.example")),
        "{:?}",
        elsewhere.notes
    );

    let korean = from_pages(
        &[board()],
        &Options {
            lang: "ko".into(),
            ..options(vec![Schema {
                url: "https://app.example/graphql".into(),
                json: schema_json(),
            }])
        },
    )
    .unwrap();
    assert!(
        korean.yaml.contains("desc: notices 조회"),
        "{}",
        korean.yaml
    );
}

#[test]
fn a_recorded_call_names_the_endpoint_and_the_schema_adds_the_rest() {
    let har = json!({"log": {"entries": [{
        "request": {"method": "POST", "url": "https://app.example/graphql?op=notices", "headers": [],
            "postData": {"mimeType": "application/json", "text": json!({
                "operationName": "notices", "query": "query notices { notices { id } }", "variables": {}
            }).to_string()}},
        "response": {"status": 200, "content": {"mimeType": "application/json", "text": "{\"data\": {\"notices\": []}}"}}
    }, {
        "request": {"method": "POST", "url": "https://app.example/api/items", "headers": [],
            "postData": {"mimeType": "application/json", "text": "{\"q\": \"x\"}"}},
        "response": {"status": 200, "content": {"mimeType": "application/json", "text": "{}"}}
    }]}});
    let text = serde_json::to_vec(&har).unwrap();
    assert_eq!(
        endpoints_in_recording(&text),
        ["https://app.example/graphql"]
    );
    let draft = from_har(
        &text,
        &options(vec![Schema {
            url: "https://app.example/graphql".into(),
            json: schema_json(),
        }]),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "create-item",
            "notices",
            "notices-2",
            "notice",
            "search-notices",
            "notices-by-ids",
            "me",
            "version",
            "wide",
            "search"
        ],
        "the recorded notices keeps its name; the schema's gets the next one"
    );
}

#[test]
fn the_introspection_query_is_a_read() {
    let spec = itda_spec::parse(
        json!({
            "spec_version": 1,
            "site": {"name": "gql", "base_url": "https://app.example"},
            "operations": [{
                "name": "schema", "method": "POST", "path": "/graphql", "effect": "read",
                "evidence": "the standard introspection query",
                "body": {"json": {"query": {"type": "string", "default": INTROSPECTION_QUERY}}}
            }]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        itda_policy::operation_effect(spec.op("schema").unwrap(), &BTreeMap::new()),
        Effect::Read
    );
    assert!(spec.op("schema").unwrap().graphql_query(&BTreeMap::new()));
}
