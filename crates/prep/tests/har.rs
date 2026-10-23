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
        notes.contains("a preflight step reads _csrf fresh from /board/writeForm.do"),
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
        notes.contains("a preflight step reads X-CSRF-Token fresh from the csrf-token tag"),
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

/// A site that keeps its session in the address: the sign-in answers with
/// a redirect carrying `sid`, and every page and call after it sends it.
fn url_session_recording(redirect_carries_it: bool) -> serde_json::Value {
    let token = "9f8e7d6c5b4a3210fedc";
    let location = if redirect_carries_it {
        format!("/portal/main.do?sid={token}")
    } else {
        "/portal/main.do".to_owned()
    };
    let page = |url: String, html: &str| {
        serde_json::json!({
            "request": {"method": "GET", "url": url, "headers": [], "queryString": []},
            "response": {"status": 200, "headers": [], "content": {"mimeType": "text/html", "text": html}}
        })
    };
    serde_json::json!({"log": {"entries": [
        {
            "request": {"method": "GET", "url": "https://portal.example.com/login?returnTo=%2Fportal", "headers": []},
            "response": {"status": 200, "headers": [], "content": {"mimeType": "text/html", "text":
                "<title>Sign in</title><form method='post' action='/login'><input name='user_id'><input type='password' name='pw'></form>"}}
        },
        {
            "request": {"method": "POST", "url": "https://portal.example.com/login", "headers": [],
                "postData": {"mimeType": "application/x-www-form-urlencoded", "text": "user_id=kim&pw=PW-VALUE"}},
            "response": {"status": 302, "headers": [{"name": "Location", "value": location}],
                "content": {"mimeType": "text/html", "text": ""}}
        },
        page(format!("https://portal.example.com/portal/main.do?sid={token}"),
            &format!("<title>Main</title><h1>Notices</h1><table><thead><tr><th>No</th><th>Title</th></tr></thead>
                <tr><td>2</td><td><a href='/portal/view.do?sid={token}&id=2'>Second</a></td></tr>
                <tr><td>1</td><td><a href='/portal/view.do?sid={token}&id=1'>First</a></td></tr></table>")),
        {
            "request": {"method": "GET", "url": format!("https://portal.example.com/api/notices?sid={token}&page=1"), "headers": [], "queryString": [{"name": "sid", "value": token}, {"name": "page", "value": "1"}]},
            "response": {"status": 200, "headers": [], "content": {"mimeType": "application/json", "text": "{\"items\": [], \"total\": 2}"}}
        },
        {
            "request": {"method": "GET", "url": format!("https://portal.example.com/api/notices/2?sid={token}"), "headers": []},
            "response": {"status": 200, "headers": [], "content": {"mimeType": "application/json", "text": "{\"id\": 2, \"title\": \"t\"}"}}
        },
        {
            "request": {"method": "POST", "url": format!("https://portal.example.com/api/notices?sid={token}"), "headers": [],
                "postData": {"mimeType": "application/json", "text": "{\"title\": \"typed\"}"}},
            "response": {"status": 302, "headers": [{"name": "Location", "value": format!("/portal/view.do?sid={token}&id=3")}], "content": {"mimeType": "text/html", "text": ""}}
        }
    ]}})
}

#[test]
fn a_session_the_site_carries_in_the_query_becomes_a_browser_entry_on_that_key() {
    for redirect_carries_it in [true, false] {
        let har = url_session_recording(redirect_carries_it);
        let text = serde_json::to_vec(&har).unwrap();
        assert_eq!(
            itda_prep::query_session_key(har["log"]["entries"].as_array().unwrap()).as_deref(),
            Some("sid"),
            "redirect carries it: {redirect_carries_it}"
        );
        let draft = itda_prep::from_recording(&text, &Options::default(), "`itda record`").unwrap();
        assert!(draft.problems.is_empty(), "{:?}", draft.problems);
        let everything = format!("{}\n{}", draft.yaml, draft.notes.join("\n"));
        assert!(!everything.contains("9f8e7d6c5b4a3210fedc"), "{everything}");
        assert!(!everything.contains("PW-VALUE"), "{everything}");
        let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
        itda_policy::validate_spec(&spec).unwrap();
        assert_eq!(spec.auth.len(), 1, "{}", draft.yaml);
        let session = &spec.auth[0];
        assert_eq!(
            (session.name.as_str(), session.kind.as_str()),
            ("session", "browser")
        );
        assert_eq!(session.value_from, "query:sid");
        let login = session.login.as_ref().unwrap();
        assert_eq!(
            (
                login.path.as_str(),
                login.user.as_str(),
                login.password.as_str()
            ),
            ("/login", "user_id", "pw")
        );
        for op in spec.operations.iter().filter(|op| op.path != "/login") {
            assert_eq!(op.auth, "session", "{}: {}", op.name, draft.yaml);
            assert!(!op.query.contains_key("sid"), "{}: {}", op.name, draft.yaml);
        }
        assert!(
            spec.op("post-login").is_err(),
            "the sign-in is a person's:\n{}",
            draft.yaml
        );
        assert!(
            draft
                .notes
                .iter()
                .any(|n| n.starts_with("POST /login: the sign-in; left out.")),
            "{:?}",
            draft.notes
        );
        let list = spec.op("list-notices").unwrap();
        assert!(list.query["page"].required);
        let main = spec.op("get-main").unwrap();
        assert_eq!(main.path, "/portal/main.do");
        assert!(main.query.is_empty(), "{}", draft.yaml);
        assert!(
            draft
                .notes
                .iter()
                .any(|n| n.contains("carry the sid query parameter")),
            "{:?}",
            draft.notes
        );
        let in_recording = itda_prep::pages_in_recording(&text, "https://portal.example.com");
        assert!(
            in_recording.contains(&"https://portal.example.com/portal/main.do".to_owned()),
            "{in_recording:?}"
        );
        assert!(
            in_recording.iter().all(|url| !url.contains("sid=")),
            "no page address keeps the session: {in_recording:?}"
        );

        // The copy kept for a refresh has the session blanked everywhere it
        // appeared: addresses, the redirect, the links in the page.
        let kept = String::from_utf8(redacted_recording(&text).unwrap()).unwrap();
        assert!(!kept.contains("9f8e7d6c5b4a3210fedc"), "{kept}");
        assert!(
            kept.contains("sid=%3Credacted%3E") || kept.contains("sid=<redacted>"),
            "{kept}"
        );
        assert!(kept.contains("page=1"), "other values stay: {kept}");
        // The kept copy still drafts the query session and the same API
        // actions. The sign-in page's HTML is wiped for privacy, so the
        // login form and the login page it gave cannot come back from it;
        // that is the redaction working, not a regression.
        let same = itda_prep::from_recording(kept.as_bytes(), &Options::default(), "`itda record`")
            .unwrap();
        let same_spec = itda_spec::parse(same.yaml.as_bytes()).unwrap();
        assert_eq!(
            same_spec.find_auth("session").unwrap().value_from,
            "query:sid"
        );
        let api = |s: &itda_spec::Spec| -> Vec<String> {
            s.operations
                .iter()
                .filter(|op| op.path != "/login")
                .map(|op| format!("{} {} {}", op.name, op.path, op.auth))
                .collect()
        };
        assert_eq!(
            api(&same_spec),
            api(&spec),
            "the API actions draft the same"
        );
    }

    // Without a sign-in, no key is a session, however it looks.
    let mut har = url_session_recording(true);
    har["log"]["entries"].as_array_mut().unwrap().drain(..2);
    assert_eq!(
        itda_prep::query_session_key(har["log"]["entries"].as_array().unwrap()),
        None
    );
}

#[test]
fn a_bearer_the_site_keeps_in_storage_becomes_a_refreshable_session() {
    use itda_prep::StorageAuth;
    const JWT: &str =
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1LTEiLCJleHAiOjQxMDI0NDQ4MDB9.sig";
    let har = serde_json::json!({"log": {"entries": [
        {"request": {"method": "GET", "url": "https://app.example/api/items",
                     "headers": [{"name": "Authorization", "value": format!("Bearer {JWT}")}]},
         "response": {"status": 200, "content": {"mimeType": "application/json", "text": "{\"items\": []}"}}}
    ]}});
    let draft = from_har(
        &serde_json::to_vec(&har).unwrap(),
        &Options {
            name: Some("app".into()),
            // The sign-in window showed the token is kept in localStorage.
            storage: Some(StorageAuth {
                key: "access_token".into(),
                header: "Authorization".into(),
                prefix: "Bearer ".into(),
            }),
            ..Options::default()
        },
    )
    .unwrap();
    // The action uses the session, and the session is a header credential
    // whose value the keyring holds under the storage key, not a static
    // store: token. The token itself never reaches the draft.
    assert!(
        draft.yaml.contains("value_from: storage:access_token"),
        "{}",
        draft.yaml
    );
    assert!(draft.yaml.contains("kind: header"), "{}", draft.yaml);
    assert!(
        draft.yaml.contains("prefix: 'Bearer '") || draft.yaml.contains("prefix: \"Bearer \""),
        "{}",
        draft.yaml
    );
    assert!(draft.yaml.contains("auth: session"), "{}", draft.yaml);
    assert!(!draft.yaml.contains("store:app"), "{}", draft.yaml);
    assert!(
        !draft.yaml.contains(JWT),
        "the token must not be in the draft: {}",
        draft.yaml
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("keeps in the browser") && n.contains("access_token")),
        "{:?}",
        draft.notes
    );

    // Without the storage evidence, the same bearer is a plain store: token.
    let plain = from_har(
        &serde_json::to_vec(&har).unwrap(),
        &Options {
            name: Some("app".into()),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(
        plain.yaml.contains("value_from: store:app/token"),
        "{}",
        plain.yaml
    );
}
