#![allow(clippy::unwrap_used)]
use itda_prep::{Options, from_har};

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
