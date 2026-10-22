#![allow(clippy::unwrap_used)]
use itda_prep::{Options, from_har, redacted_recording};

fn recording() -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/intranet.har",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn a_recording_becomes_a_valid_draft() {
    let draft = from_har(&recording(), &Options::default()).unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(draft.site, "intranet-example");
    assert!(draft.yaml.starts_with("# Drafted by `itda draft har`"));
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.site.base_url, "https://intranet.example.com");

    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        ["list-notices", "create-notice", "get-notice", "post-write"]
    );
    assert_eq!((draft.operations, draft.reads), (4, 2));

    let list = spec.op("list-notices").unwrap();
    assert_eq!(list.auth, "token");
    assert!(list.query["page"].required && list.query["page"].kind == "int");
    assert!(!list.query["q"].required, "only one sample searched");
    assert!(
        !list.query.contains_key("_"),
        "cache busters are not inputs"
    );
    let response = list.response.as_ref().unwrap();
    assert_eq!(response.required_pointers, ["/items", "/total"]);
    assert!(list.evidence.contains("seen 2 times"));

    let get = spec.op("get-notice").unwrap();
    assert_eq!(get.path, "/api/v1/notices/{notice_id}");
    assert_eq!(
        get.response.as_ref().unwrap().required_pointers,
        ["/id", "/title"]
    );

    let create = spec.op("create-notice").unwrap();
    let body = &create.body.as_ref().unwrap().json;
    assert_eq!(body["views"].kind, "int");
    assert_eq!(body["pinned"].kind, "bool");
    assert!(!body.contains_key("csrf_token"));

    // The form post sent only the session cookie, so it uses the browser sign-in.
    let write = spec.op("post-write").unwrap();
    assert_eq!(write.auth, "session");
    assert!(write.body.as_ref().unwrap().form["subject"].required);

    assert_eq!(spec.auth.len(), 2);
    assert_eq!(spec.auth[0].header, "Authorization");
    assert_eq!(spec.auth[0].prefix, "Bearer ");
    assert_eq!(spec.auth[0].value_from, "store:intranet-example/token");
    assert_eq!(spec.auth[1].kind, "browser");
    assert_eq!(spec.auth[1].value_from, "cookie:SID");
}

#[test]
fn no_recorded_value_reaches_the_draft() {
    let draft = from_har(&recording(), &Options::default()).unwrap();
    let everything = format!("{}\n{}", draft.yaml, draft.notes.join("\n"));
    for secret in [
        "S3CR3T",
        "SESSION-COOKIE",
        "PRIVATE",
        "CSRF-",
        "hello",
        "kim",
        "1700000000",
    ] {
        assert!(
            !everything.contains(secret),
            "{secret} leaked:\n{everything}"
        );
    }
}

#[test]
fn what_was_left_out_is_listed() {
    let notes = from_har(&recording(), &Options::default())
        .unwrap()
        .notes
        .join("\n");
    assert!(notes.contains("analytics.example.org"), "{notes}");
    assert!(
        notes.contains("itda login intranet-example --browser"),
        "{notes}"
    );
    assert!(notes.contains("POST /api/v1/files: skipped"), "{notes}");
    assert!(notes.contains("left out csrf_token"), "{notes}");
    assert!(
        notes.contains("_csrf come from /board/writeForm.do through a preflight step"),
        "{notes}"
    );
}

#[test]
fn a_form_token_is_read_from_the_form_page_before_each_post() {
    let draft = from_har(&recording(), &Options::default()).unwrap();
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();
    let post = spec.op("post-write").unwrap();
    assert_eq!(post.preflight.len(), 1);
    assert_eq!(post.preflight[0].path, "/board/writeForm.do");
    assert_eq!(
        post.preflight[0].extract["csrf"],
        "css:input[name=\"_csrf\"]@value"
    );
    let form = &post.body.as_ref().unwrap().form;
    assert!(form["_csrf"].from_step());
    assert!(!draft.yaml.contains("PRIVATE-BBS"));
}

#[test]
fn options_and_bad_input() {
    let narrowed = from_har(
        &recording(),
        &Options {
            name: Some("Board".into()),
            include: Some("write".into()),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!((narrowed.site.as_str(), narrowed.operations), ("board", 1));

    let under_api = from_har(
        &recording(),
        &Options {
            base_url: Some("https://intranet.example.com/api/v1/".into()),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(under_api.yaml.contains("path: /notices/{notice_id}"));
    assert_eq!(under_api.operations, 3);

    assert!(from_har(b"{\"log\": {}}", &Options::default()).is_err());
    assert!(from_har(b"not json", &Options::default()).is_err());
    assert!(from_har(br#"{"log":{"entries":[]}}"#, &Options::default()).is_err());
}

#[test]
fn a_header_token_is_read_from_the_page_meta_tag() {
    let entry = |method: &str, url: &str, headers: serde_json::Value, mime: &str, text: &str| {
        serde_json::json!({
            "request": {"method": method, "url": url, "headers": headers},
            "response": {"status": 200, "content": {"mimeType": mime, "text": text}}
        })
    };
    let page = "<html><head><meta charset=utf-8><meta name=\"csrf-token\" content=\"CSRF-SECRET\"></head></html>";
    let har = serde_json::json!({"log": {"entries": [
        entry("GET", "https://app.example.com/posts/new?draft=PRIVATE", serde_json::json!([]), "text/html", page),
        entry(
            "POST",
            "https://app.example.com/api/posts",
            serde_json::json!([
                {"name": "X-CSRF-Token", "value": "CSRF-SECRET"},
                {"name": "Cookie", "value": "_app_session=SESSION-SECRET"}
            ]),
            "application/json",
            "{\"id\": 1}"
        ),
        entry(
            "DELETE",
            "https://app.example.com/api/posts/1",
            serde_json::json!([{"name": "X-XSRF-TOKEN", "value": "CSRF-SECRET"}]),
            "application/json",
            "{}"
        ),
    ]}});
    let draft = from_har(&serde_json::to_vec(&har).unwrap(), &Options::default()).unwrap();
    assert!(!draft.yaml.contains("SECRET") && !draft.yaml.contains("PRIVATE"));
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();
    let create = spec.op("create-post").unwrap();
    assert_eq!(create.headers["X-CSRF-Token"], "{{step.page.token}}");
    assert_eq!(create.preflight[0].path, "/posts/new");
    assert_eq!(
        create.preflight[0].extract["token"],
        "css:meta[name=\"csrf-token\"]@content"
    );
    let notes = draft.notes.join("\n");
    assert!(
        notes.contains("X-CSRF-Token comes from the csrf-token tag"),
        "{notes}"
    );
}

#[test]
fn graphql_operations_become_one_action_each() {
    let call = |name: &str, query: &str, variables: serde_json::Value, answer: &str| {
        serde_json::json!({
            "request": {"method": "POST", "url": "https://app.example/api/graphql", "headers": [
                {"name": "Content-Type", "value": "application/json"}
            ], "postData": {"mimeType": "application/json", "text": serde_json::json!({
                "operationName": name, "query": query, "variables": variables
            }).to_string()}},
            "response": {"status": 200, "content": {"mimeType": "application/json", "text": answer}}
        })
    };
    let har = serde_json::json!({"log": {"entries": [
        call("exploreProjects", "query exploreProjects($first: Int, $search: String, $after: String, $filter: Filter) { projects(first: $first) { nodes { id } } }",
             serde_json::json!({"first": 20, "search": "", "after": null, "filter": {"active": true}}),
             "{\"data\": {\"projects\": {\"nodes\": []}}}"),
        call("exploreProjects", "query exploreProjects($first: Int, $search: String, $after: String, $filter: Filter) { projects(first: $first) { nodes { id } } }",
             serde_json::json!({"first": 40, "after": "abc", "filter": {"active": true}}),
             "{\"data\": {\"projects\": {\"nodes\": []}}}"),
        call("createIssue", "mutation createIssue($title: String!) { createIssue(title: $title) { id } }",
             serde_json::json!({"title": "typed by a person"}),
             "{\"data\": {\"createIssue\": {\"id\": 1}}}"),
    ]}});
    let draft = from_har(&serde_json::to_vec(&har).unwrap(), &Options::default()).unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert!(!draft.yaml.contains("typed by a person"));
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["create-issue", "explore-projects"]);

    let explore = spec.op("explore-projects").unwrap();
    assert_eq!(
        itda_policy::operation_effect(explore, &std::collections::BTreeMap::new()),
        itda_spec::Effect::Read
    );
    let body = explore.body.as_ref().unwrap();
    assert!(
        body.json["query"]
            .default
            .as_ref()
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("query exploreProjects")
    );
    assert_eq!(body.json["first"].kind, "int");
    assert!(body.json["first"].required);
    assert!(!body.json["search"].required, "only one sample sent it");
    assert!(!body.json.contains_key("filter"), "objects are not inputs");
    let template = body.json_template.as_ref().unwrap();
    assert_eq!(template["variables"]["first"], "{{first}}");
    assert_eq!(template["operationName"], "{{operationName}}");
    assert_eq!(
        explore.response.as_ref().unwrap().required_pointers,
        ["/data/projects"]
    );
    assert!(draft.notes.iter().any(|n| n.contains("filter")));

    let create = spec.op("create-issue").unwrap();
    assert_eq!(
        itda_policy::operation_effect(create, &std::collections::BTreeMap::new()),
        itda_spec::Effect::Write
    );
    assert!(create.body.as_ref().unwrap().json["title"].required);
}

#[test]
fn the_redacted_copy_keeps_names_and_shapes_and_drafts_the_same() {
    let original = recording();
    let redacted = redacted_recording(&original).unwrap();
    let text = String::from_utf8(redacted.clone()).unwrap();
    for value in ["S3CR3T-TOKEN-VALUE", "SESSION-COOKIE-VALUE", "CSRF-VALUE"] {
        assert!(!text.contains(value), "{value} kept:\n{text}");
    }
    assert!(text.contains("Bearer <redacted>"), "{text}");
    assert!(text.contains("SID=<redacted>"), "{text}");
    assert!(text.contains(r#"\"csrf_token\":\"<redacted>\""#), "{text}");
    assert!(
        !text.contains(r#""cookies":null"#),
        "no field was added:\n{text}"
    );
    assert!(text.contains("_csrf=%3Credacted%3E"), "{text}");
    // Values that are not credentials stay, so types are still inferred.
    assert!(text.contains("page=2"), "{text}");
    let same = from_har(&redacted, &Options::default()).unwrap();
    let before = from_har(&original, &Options::default()).unwrap();
    assert_eq!(same.yaml, before.yaml);
    assert_eq!(same.notes, before.notes);
}

#[test]
fn a_sign_in_call_loses_its_answer_in_the_redacted_copy() {
    let har = serde_json::json!({"log": {"entries": [
        {
            "request": {"method": "POST", "url": "https://a.example/login", "headers": [],
                "postData": {"mimeType": "application/x-www-form-urlencoded", "text": "user=kim&password=PW-VALUE"}},
            "response": {"status": 200, "headers": [{"name": "Set-Cookie", "value": "SID=NEW-VALUE; Path=/"}],
                "content": {"mimeType": "text/html", "text": "<html>welcome kim, your token is TOK-VALUE</html>"}}
        },
        {
            "request": {"method": "POST", "url": "https://a.example/api/session", "headers": [],
                "postData": {"mimeType": "application/json", "text": "{\"email\": \"kim@a.example\", \"passwd\": \"PW2\", \"nested\": {\"api_key\": \"NK\"}}"}},
            "response": {"status": 200, "headers": [], "content": {"mimeType": "application/json", "text": "{\"token\": \"TOK2\"}"}}
        },
        {
            "request": {"method": "GET", "url": "https://a.example/api/items", "headers": [{"name": "X-Session-Id", "value": "S-VALUE"}]},
            "response": {"status": 200, "headers": [], "content": {"mimeType": "application/json", "text": "{\"items\": [1]}"}}
        }
    ]}});
    let text = String::from_utf8(redacted_recording(har.to_string().as_bytes()).unwrap()).unwrap();
    for value in [
        "PW-VALUE",
        "TOK-VALUE",
        "NEW-VALUE",
        "PW2",
        "NK",
        "TOK2",
        "S-VALUE",
    ] {
        assert!(!text.contains(value), "{value} kept:\n{text}");
    }
    assert!(
        text.contains("kim%40a.example") || text.contains("kim@a.example"),
        "{text}"
    );
    assert!(
        text.contains("\\\"items\\\": [1]"),
        "the read's answer stays:\n{text}"
    );
    assert!(text.contains("SID=<redacted>"), "{text}");
    assert!(redacted_recording(b"not json").is_err());
}

#[test]
fn a_recorded_page_number_declares_page_turning() {
    let draft = from_har(&recording(), &Options::default()).unwrap();
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let list = spec.op("list-notices").unwrap();
    let pagination = list.pagination.as_ref().unwrap();
    assert_eq!(
        (pagination.style.as_str(), pagination.next.as_str()),
        ("page", "")
    );
    assert_eq!(pagination.input(), ("page", false));
    assert!(spec.op("get-notice").unwrap().pagination.is_none());
    assert!(spec.op("create-notice").unwrap().pagination.is_none());
}
