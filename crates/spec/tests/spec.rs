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
    kind: derived
    header: X-CSRF-Token
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
