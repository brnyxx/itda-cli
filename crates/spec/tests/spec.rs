//! Derived from Hycli `src/spec_test.rs` (Apache-2.0). Responses use a plain
//! test type instead of the network layer's response.
#![allow(clippy::unwrap_used)]
use std::collections::BTreeMap;

use itda_spec as spec;
use itda_spec::ResponseLike;
use itda_types::error as apperr;

struct TestResponse {
    url: &'static str,
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    bytes: Vec<u8>,
}
impl ResponseLike for TestResponse {
    fn url(&self) -> &str {
        self.url
    }
    fn status(&self) -> u16 {
        self.status
    }
    fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| *v)
            .unwrap_or("")
    }
    fn body(&self) -> &[u8] {
        &self.bytes
    }
    fn successful(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

const GOOD: &str = r##"
spec_version: 1
site:
  name: t1
  title: Test One
  base_url: https://api.example.com
auth:
  - {name: public, kind: none}
  - {name: bearer, kind: header, header: Authorization, prefix: "Bearer ", value_from: "store:t1/token"}
operations:
  - name: item
    method: GET
    path: /items/{id}
    auth: public
    params: {id: {type: int, required: true}}
    query: {verbose: {type: bool}, limit: {type: int, default: 10}}
  - name: create
    method: POST
    path: /items
    auth: bearer
    body:
      json: {name: {type: string, required: true}, count: {type: int, default: 1}}
models:
  item_view:
    id: {type: int}
    name: {type: string}
"##;

fn parse(s: &str) -> spec::Spec {
    spec::parse(s.as_bytes()).unwrap()
}

#[test]
fn parse_validate_good() {
    let sp = parse(GOOD);
    assert!(sp.validate().is_empty(), "issues: {:?}", sp.validate());
}

#[test]
fn validate_catches() {
    let bad = r##"
spec_version: 1
site: {name: t2, title: T, base_url: https://x.example.com}
auth:
  - {name: public, kind: none}
operations:
  - name: a
    method: GET
    path: /x/{id}
    auth: ghost
  - name: b
    method: GET
    path: /y
    auth: public
    query: {limit: {type: int}}
    body:
      json: {limit: {type: int}}
"##;
    let sp = parse(bad);
    let issues = sp.validate();
    let joined = issues
        .iter()
        .map(|i| format!("{} {}", i.field, i.detail))
        .collect::<String>();
    assert!(joined.contains("params"), "missing params issue: {joined}");
    assert!(joined.contains("ghost"), "missing auth issue: {joined}");
    assert!(
        joined.contains("conflict"),
        "missing conflict issue: {joined}"
    );
}

#[test]
fn build_request() {
    let sp = parse(GOOD);
    let op = sp.op("item").unwrap();
    let mut args = BTreeMap::new();
    args.insert("id".to_string(), "42".to_string());
    let (url, body) = sp.build_request(op, &args).unwrap();
    assert_eq!(url, "https://api.example.com/items/42?limit=10");
    assert!(body.is_none());

    let err = sp.build_request(op, &Default::default()).unwrap_err();
    assert!(err.msg.contains("id"), "missing required message: {err}");

    let op2 = sp.op("create").unwrap();
    let mut args = BTreeMap::new();
    args.insert("name".to_string(), "logo".to_string());
    args.insert("count".to_string(), "3".to_string());
    let (_, body2) = sp.build_request(op2, &args).unwrap();
    let m: serde_json::Value = serde_json::from_slice(&body2.unwrap()).unwrap();
    assert_eq!(m["name"], "logo");
    assert!(m["count"].is_number());
}

#[test]
fn apply_model() {
    let sp = parse(GOOD);
    let raw = br#"{"id": 7, "name": "abc", "extra": true}"#;
    let out = sp.apply_model("item_view", raw).unwrap();
    assert_eq!(out["id"], 7);
    assert_eq!(out["name"], "abc");
    assert!(
        out.get("extra").is_none(),
        "unexpected extra field in model output"
    );
}

#[test]
fn apply_model_to_each_record_of_a_list() {
    let sp = parse(GOOD);
    let raw = br#"[{"id": 1, "name": "a", "extra": 1}, {"id": 2, "name": "b"}]"#;
    let out = sp.apply_model("item_view", raw).unwrap();
    assert_eq!(out[0]["name"], "a");
    assert_eq!(out[1]["id"], 2);
    assert!(out[0].get("extra").is_none());
}

#[test]
fn op_not_found_lists_names() {
    let sp = parse(GOOD);
    let err = sp.op("nope").unwrap_err();
    assert!(err.remedy.contains("item"));
    let _ = apperr::Code::Ok;
}

#[test]
fn v2_json_template_and_response_preprocessing() {
    let raw_spec = r##"
spec_version: 2
site:
  name: v2site
  title: V2 Site
  base_url: https://app.example.com
auth:
  - name: xsrf
    kind: browser
    value_from: cookie:ct0
operations:
  - name: next-feed
    method: GET
    path: /feed
    auth: xsrf
    response:
      format: json
      ssr_json: __NEXT_DATA__
      extract_pointer: /props/pageProps/items
      required_pointers:
        - /0/id
  - name: create-post
    method: POST
    path: /api/posts
    auth: xsrf
    preflight:
      - name: csrf
        path: /feed
        extract:
          token: "css:meta[name='csrf-token']@content"
    body:
      json:
        title: {type: string, required: true}
        limit: {type: int, default: 5}
      json_template:
        operationName: CreatePost
        variables:
          input:
            title: "{{title}}"
            limit: "{{limit}}"
            csrf: "{{step.csrf.token}}"
    response:
      format: json
      strip_prefix: ")]}'\n"
      extract_pointer: /data/post
"##;
    let sp = parse(raw_spec);
    assert!(
        sp.validate().is_empty(),
        "v2 validate issues: {:?}",
        sp.validate()
    );

    let create_op = sp.op("create-post").unwrap();
    let mut args = BTreeMap::new();
    args.insert("title".into(), "Hello".into());
    args.insert("step.csrf.token".into(), "tok-123".into());
    let (_, body_bytes) = sp.build_request(create_op, &args).unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes.unwrap()).unwrap();
    assert_eq!(body_json["operationName"], "CreatePost");
    assert_eq!(body_json["variables"]["input"]["title"], "Hello");
    assert_eq!(body_json["variables"]["input"]["limit"], 5);
    assert_eq!(body_json["variables"]["input"]["csrf"], "tok-123");

    let xssi_resp = TestResponse {
        url: "https://app.example.com/api/posts",
        status: 200,
        headers: vec![("content-type", "application/json")],
        bytes: b")]}'\n{\"data\":{\"post\":{\"id\":\"p-1\"}}}".to_vec(),
    };
    assert!(create_op.matches_response(&xssi_resp));
    assert_eq!(
        create_op.parse_json_response(&xssi_resp).unwrap()["id"],
        "p-1"
    );

    let next_op = sp.op("next-feed").unwrap();
    let ssr_resp = TestResponse {
        url: "https://app.example.com/feed",
        status: 200,
        headers: vec![("content-type", "text/html")],
        bytes: br#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"items":[{"id":"clip-7"}]}}}</script></body></html>"#.to_vec(),
    };
    assert!(next_op.matches_response(&ssr_resp));
    assert_eq!(
        next_op.parse_json_response(&ssr_resp).unwrap()[0]["id"],
        "clip-7"
    );
}

#[test]
fn html_expectations_require_declared_fields_and_content_type() {
    let sp = parse(
        r##"
site: {name: h1, base_url: https://example.com}
operations:
  - name: profile
    method: GET
    path: /me
    effect: read
    evidence: synthetic
    response:
      format: html
      required_html_fields:
        - {selector: "h1.name"}
"##,
    );
    let op = sp.op("profile").unwrap();
    let page = |body: &str, ct: &'static str| TestResponse {
        url: "https://example.com/me",
        status: 200,
        headers: vec![("content-type", ct)],
        bytes: body.as_bytes().to_vec(),
    };
    assert!(op.matches_response(&page("<h1 class=name>Mina</h1>", "text/html")));
    assert!(!op.matches_response(&page("<h1 class=name>Mina</h1>", "application/json")));
    assert!(!op.matches_response(&page("<h1>Sign in</h1>", "text/html")));
    let mut failed = page("<h1 class=name>Mina</h1>", "text/html");
    failed.status = 403;
    assert!(!op.matches_response(&failed));
    assert_eq!(op.effect, spec::Effect::Read);
}

#[test]
fn a_time_is_read_from_a_short_string() {
    use std::time::Duration;
    assert_eq!(spec::duration("500ms"), Some(Duration::from_millis(500)));
    assert_eq!(spec::duration(" 2s "), Some(Duration::from_secs(2)));
    assert_eq!(spec::duration("1.5s"), Some(Duration::from_millis(1500)));
    assert_eq!(spec::duration("5"), None);
    assert_eq!(spec::duration("soon"), None);
    assert_eq!(spec::duration(""), None);
    let parsed = parse(
        "spec_version: 1\nsite: {name: s, base_url: https://example.test}\ndefaults: {interval: 2s}\noperations: []\n",
    );
    assert_eq!(parsed.defaults.interval(), Some(Duration::from_secs(2)));
}

#[test]
fn an_optional_template_input_left_out_leaves_the_body() {
    let sp = parse(
        r##"
spec_version: 1
site: {name: gql, base_url: https://app.example}
operations:
  - name: notices
    method: POST
    path: /graphql
    effect: read
    evidence: doc
    body:
      json:
        query: {type: string, default: "query Notices($first: Int) { notices(first: $first) { id } }"}
        first: {type: int}
        after: {type: string}
      json_template:
        query: "{{query}}"
        variables: {first: "{{first}}", after: "{{after}}", page: "p-{{after}}"}
"##,
    );
    let op = sp.op("notices").unwrap();
    let (_, body) = sp.build_request(op, &BTreeMap::new()).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body.unwrap()).unwrap();
    assert_eq!(body["variables"], serde_json::json!({"page": "p-"}));
    let mut args = BTreeMap::new();
    args.insert("first".to_owned(), "3".to_owned());
    let (_, body) = sp.build_request(op, &args).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body.unwrap()).unwrap();
    assert_eq!(
        body["variables"],
        serde_json::json!({"first": 3, "page": "p-"})
    );
}

#[test]
fn pagination_is_parsed_and_checked() {
    let good = parse(
        r#"
spec_version: 1
site: {name: paged, base_url: https://paged.example}
operations:
  - name: list
    method: GET
    path: /items
    effect: read
    evidence: doc
    query:
      cursor: {type: string}
    pagination:
      style: cursor
      next: /meta/next_cursor
      send: {query: cursor}
  - name: pages
    method: GET
    path: /pages
    effect: read
    evidence: doc
    query:
      page: {type: int, default: 1}
    pagination: {style: page, next: "header:link", send: {query: page}}
"#,
    );
    assert!(good.validate().is_empty(), "{:?}", good.validate());
    let list = good.op("list").unwrap();
    let pagination = list.pagination.as_ref().unwrap();
    assert_eq!(pagination.input(), ("cursor", false));
    assert!(pagination.hides_input());
    assert!(list.hidden_input("cursor"));
    assert!(!good.op("pages").unwrap().hidden_input("page"));
    // The marker input is written back as given.
    let text = serde_norway::to_string(&good).unwrap();
    assert!(text.contains("next: /meta/next_cursor"), "{text}");
    assert!(!text.contains("more:"), "{text}");

    let bad = parse(
        r#"
spec_version: 1
site: {name: paged, base_url: https://paged.example}
operations:
  - name: a
    method: GET
    path: /a
    effect: read
    evidence: doc
    pagination: {style: scroll, next: meta.next, send: {query: cursor}}
  - name: b
    method: GET
    path: /b
    effect: read
    evidence: doc
    query: {cursor: {type: string, default: x}}
    pagination: {style: cursor, send: {query: cursor, body: cursor}}
  - name: c
    method: GET
    path: /c
    effect: read
    evidence: doc
    query: {page: {type: string}}
    pagination: {style: page, next: "header:link", send: {body: page}}
"#,
    );
    let details: Vec<String> = bad
        .validate()
        .into_iter()
        .map(|issue| format!("{}: {}", issue.field, issue.detail))
        .collect();
    let text = details.join("\n");
    for expected in [
        "operations.a.pagination: unsupported style: scroll",
        "operations.a.pagination: next must be a JSON pointer",
        "operations.a.pagination: send names cursor, which is not declared under query",
        "operations.b.pagination: a cursor read must say where the next cursor is",
        "operations.b.pagination: send must name one input",
        "operations.c.pagination: header:link carries a query value",
        "operations.c.pagination: send names page, which is not declared under body.json",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
}

#[test]
fn a_change_may_name_the_read_that_confirms_it() {
    let good = parse(
        r#"
spec_version: 1
site: {name: board, base_url: https://board.example}
operations:
  - name: list-notices
    method: GET
    path: /api/notices
    effect: read
    evidence: doc
    query:
      q: {type: string}
    response: {format: json, required_pointers: [/items]}
  - name: create-notice
    method: POST
    path: /api/notices
    effect: write
    evidence: doc
    body:
      json:
        title: {type: string, required: true}
    confirm:
      read: list-notices
      match: {pointer: /items/*/title, from: body.title}
"#,
    );
    assert!(good.validate().is_empty(), "{:?}", good.validate());
    let confirm = good.op("create-notice").unwrap().confirm.as_ref().unwrap();
    assert_eq!(confirm.read, "list-notices");
    assert_eq!(confirm.matching.pointer, "/items/*/title");
    assert_eq!(confirm.matching.input(), Some(("body", "title")));
    assert!(good.op("list-notices").unwrap().confirm.is_none());
    // Written back as given, under `match`.
    let text = serde_norway::to_string(&good).unwrap();
    assert!(text.contains("match:"), "{text}");
    assert!(text.contains("from: body.title"), "{text}");
    assert_eq!(text.matches("confirm:").count(), 1, "{text}");

    let bad = parse(
        r#"
spec_version: 1
site: {name: board, base_url: https://board.example}
operations:
  - name: list
    method: GET
    path: /items
    effect: read
    evidence: doc
    confirm:
      read: list
      match: {pointer: /items/*/title, from: body.title}
  - name: one
    method: GET
    path: /items/{id}
    params: {id: {type: int}}
    effect: read
    evidence: doc
  - name: a
    method: POST
    path: /items
    effect: write
    evidence: doc
    body: {json: {title: {type: string, required: true}}}
    confirm:
      read: nowhere
      match: {pointer: items.title, from: title}
  - name: b
    method: POST
    path: /items
    effect: write
    evidence: doc
    body:
      json:
        title: {type: string}
        token: {type: string, default: "{{step.form.token}}"}
        flag: {type: bool, required: true}
    confirm:
      read: a
      match: {pointer: /items/*/title, from: body.title}
  - name: c
    method: POST
    path: /items
    effect: write
    evidence: doc
    body: {json: {title: {type: string, required: true}}}
    confirm:
      read: one
      match: {pointer: /title, from: body.other}
  - name: d
    method: POST
    path: /items
    effect: write
    evidence: doc
    body:
      json:
        title: {type: string, required: true}
        token: {type: string, default: "{{step.form.token}}"}
        flag: {type: bool, required: true}
    confirm:
      read: list
      match: {pointer: /items/*/token, from: body.token}
  - name: e
    method: POST
    path: /items
    effect: write
    evidence: doc
    body:
      json:
        flag: {type: bool, required: true}
    confirm:
      read: list
      match: {pointer: /items/*/flag, from: body.flag}
"#,
    );
    let details: Vec<String> = bad
        .validate()
        .into_iter()
        .map(|issue| format!("{}: {}", issue.field, issue.detail))
        .collect();
    let text = details.join("\n");
    for expected in [
        "operations.list.confirm: only a change names the read that confirms it",
        "operations.list.confirm: read must name another operation",
        "operations.a.confirm: read names nowhere, which is not defined",
        "operations.a.confirm: match.pointer must be a JSON pointer starting with /",
        "operations.a.confirm: match.from must name an input as params.<name>, query.<name> or body.<name>",
        "operations.b.confirm: a is not a read",
        "operations.b.confirm: match.from names title, which the change may be sent without",
        "operations.c.confirm: one needs id and a confirming read runs with its defaults alone",
        "operations.c.confirm: match.from names other, which is not declared under body",
        "operations.d.confirm: match.from names token, which a preflight step fills",
        "operations.e.confirm: match.from names flag, whose type bool cannot be compared",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
}

#[test]
fn preflight_extracts_from_embedded_data_with_a_key_and_a_pointer() {
    let good = parse(
        r#"
spec_version: 1
site: {name: s, base_url: https://s.example}
operations:
  - name: items
    method: GET
    path: /_next/data/{{step.page.build_id}}/items.json
    effect: read
    evidence: doc
    preflight:
      - name: page
        path: /
        extract: {build_id: "ssr:__NEXT_DATA__#/buildId", other: "ssr:auto#/props/x"}
"#,
    );
    assert!(good.validate().is_empty(), "{:?}", good.validate());
    let bad = parse(
        r#"
spec_version: 1
site: {name: s, base_url: https://s.example}
operations:
  - name: items
    method: GET
    path: /items
    effect: read
    evidence: doc
    preflight:
      - name: page
        path: /
        extract: {a: "ssr:__NEXT_DATA__", b: "ssr:#/x", c: "ssr:auto#buildId"}
"#,
    );
    assert_eq!(bad.validate().len(), 3, "{:?}", bad.validate());
}

#[test]
fn browser_transports_and_derived_credentials_are_refused() {
    let sp = parse(
        r##"
site: {name: leftover, base_url: https://example.com}
auth:
  - name: xsrf
    kind: derived
    header: X-CSRF-Token
    value_from: cookie:ct0
operations:
  - name: feed
    method: GET
    path: /feed
    transport: browser-fetch
  - name: page
    method: GET
    path: /page
    transport: browser
"##,
    );
    let issues = sp.validate();
    let fields: Vec<&str> = issues.iter().map(|i| i.field.as_str()).collect();
    assert_eq!(
        fields,
        [
            "auth.xsrf.kind",
            "operations.feed.transport",
            "operations.page.transport"
        ],
        "{issues:?}"
    );
    assert!(issues[0].detail.contains("unsupported auth kind: derived"));
    assert!(issues[1].remedy.contains("plain HTTP"));
}

#[test]
fn a_browser_session_may_live_in_a_cookie_or_a_query_key() {
    let with = |value_from: &str| {
        parse(&format!(
            r##"
site: {{name: portal, base_url: https://example.com}}
auth:
  - name: session
    kind: browser
    value_from: "{value_from}"
operations:
  - name: feed
    method: GET
    path: /feed
    auth: session
"##
        ))
        .validate()
    };
    assert!(with("cookie:SID").is_empty());
    assert!(with("query:sid").is_empty());
    assert!(with("").is_empty());
    for bad in ["query:", "query: ", "header:sid", "sid", "storage:token"] {
        let issues = with(bad);
        assert_eq!(issues.len(), 1, "{bad}: {issues:?}");
        assert_eq!(issues[0].field, "auth.session.value_from");
        assert!(issues[0].detail.contains("query:<key>"), "{issues:?}");
    }
}

#[test]
fn a_header_credential_comes_from_the_store_or_from_browser_storage() {
    let with = |value_from: &str| {
        parse(&format!(
            r##"
site: {{name: app, base_url: https://example.com}}
auth:
  - name: session
    kind: header
    header: Authorization
    prefix: "Bearer "
    value_from: "{value_from}"
operations:
  - name: feed
    method: GET
    path: /feed
    auth: session
"##
        ))
        .validate()
    };
    assert!(with("store:app/token").is_empty());
    assert!(with("storage:access_token").is_empty());
    for bad in ["storage:", "storage: ", "cookie:sid", "query:sid", ""] {
        let issues = with(bad);
        assert_eq!(issues.len(), 1, "{bad}: {issues:?}");
        assert_eq!(issues[0].field, "auth.session.value_from");
        assert!(issues[0].detail.contains("storage:<key>"), "{issues:?}");
    }
}
