//! Classification cases are derived from Hycli `src/integration_tests.rs`
//! (Apache-2.0); approval and validation cases are new.
#![allow(clippy::unwrap_used)]
use itda_policy::{
    Approvals, Effect, ExecutionRequest, ReviewInput, operation_effect, safe_read_url,
    spec_problems, validate_spec,
};
use itda_spec::{self as spec, Spec};
use itda_store::Store;
use serde_json::json;
use std::collections::BTreeMap;

fn spec_from(value: serde_json::Value) -> Spec {
    spec::parse(value.to_string().as_bytes()).unwrap()
}

fn fixture(origin: &str) -> Spec {
    spec_from(
        json!({"spec_version":1,"site":{"name":"fixture","title":"Fixture website","base_url":origin},"operations":[{"name":"read-items","method":"GET","path":"/items","effect":"read","evidence":"Synthetic fixture documentation"},{"name":"create-item","method":"POST","path":"/items","effect":"write","evidence":"Synthetic fixture documentation","body":{"json":{"title":{"type":"string","required":true}}}}]}),
    )
}

#[test]
fn defaults_and_disguised_methods_never_make_changes_look_read_only() {
    for path in [
        "/cart/add",
        "/api/deleteItem",
        "/api/%64elete",
        "/logout",
        "/toggle",
    ] {
        let mut op = fixture("https://example.com").operations.remove(0);
        op.path = path.into();
        assert_eq!(
            operation_effect(&op, &BTreeMap::new()),
            Effect::Write,
            "{path}"
        );
    }
    let mut op = fixture("https://example.com").operations.remove(0);
    op.path = "/search".into();
    op.name = "search-items".into();
    op.method = "POST".into();
    op.body = Some(spec::Body {
        json: BTreeMap::from([(
            "operation".into(),
            spec::Param {
                kind: "string".into(),
                required: false,
                default: Some(json!("delete")),
            },
        )]),
        ..Default::default()
    });
    assert_eq!(operation_effect(&op, &BTreeMap::new()), Effect::Write);
    for raw in [
        "https://example.com/api?mode=delete",
        "https://example.com/api?action=logout",
        "https://example.com/api?csrf=opaque",
    ] {
        assert!(!safe_read_url(&url::Url::parse(raw).unwrap()), "{raw}");
    }
    assert!(safe_read_url(
        &url::Url::parse("https://example.com/items?page=2").unwrap()
    ));
}

#[test]
fn reads_need_declared_effect_and_evidence() {
    let mut op = fixture("https://example.com").operations.remove(0);
    assert_eq!(operation_effect(&op, &BTreeMap::new()), Effect::Read);
    op.evidence.clear();
    assert_eq!(operation_effect(&op, &BTreeMap::new()), Effect::Unknown);
    op.evidence = "doc".into();
    op.method = "DELETE".into();
    assert_eq!(operation_effect(&op, &BTreeMap::new()), Effect::Write);
}

#[test]
fn graphql_strings_and_comments_do_not_disguise_a_mutation() {
    let mut op = fixture("https://example.com").operations.remove(0);
    op.name = "query-data".into();
    op.method = "POST".into();
    op.path = "/api".into();
    for query in [
        "query { item(search: \"#\") { id } } mutation { deleteAll }",
        "query { item(search: \"safe\") { id } }\nmutation M { change }",
        "subscription { updates }",
        "query A { item { id } } query B { item { id } }",
        "query { item(",
        "{ items { id } } broken syntax",
    ] {
        assert_ne!(
            operation_effect(&op, &BTreeMap::from([("query".into(), query.into())])),
            Effect::Read,
            "{query}"
        );
    }
    for query in [
        "query { item(search: \"mutation\") { id } }",
        "# mutation is just a comment\n{ items { id } }",
    ] {
        assert_eq!(
            operation_effect(&op, &BTreeMap::from([("query".into(), query.into())])),
            Effect::Read,
            "{query}"
        );
    }
}

#[test]
fn path_arguments_and_nested_controls_keep_changes_behind_review() {
    let mut op = fixture("https://example.com").operations.remove(0);
    op.path = "/api/{route}".into();
    op.params.insert(
        "route".into(),
        spec::Param {
            kind: "string".into(),
            required: true,
            default: None,
        },
    );
    assert_eq!(
        operation_effect(&op, &BTreeMap::from([("route".into(), "delete".into())])),
        Effect::Write
    );
    op.params.clear();
    op.path = "/search".into();
    op.method = "POST".into();
    op.name = "search-data".into();
    assert_eq!(
        operation_effect(
            &op,
            &BTreeMap::from([("options".into(), r#"{"filter":{"mode":"delete"}}"#.into())])
        ),
        Effect::Write
    );
}

fn request(inputs: &[(&str, &str)]) -> ExecutionRequest {
    ExecutionRequest {
        site_id: "fixture".into(),
        action_id: "create-item".into(),
        account_id: "acct".into(),
        inputs: inputs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        spec_hash: "spec-hash".into(),
        credential_hash: "cred-hash".into(),
        file_hash: String::new(),
    }
}

fn approvals() -> Approvals {
    Approvals::open(Store::in_memory().unwrap()).unwrap()
}

#[tokio::test]
async fn approvals_are_single_use_and_bound_to_the_exact_request() {
    let store = approvals();
    let req = request(&[("title", "Fixture")]);
    let review = store
        .request_approval(
            req.clone(),
            "Fixture".into(),
            "Create item".into(),
            "Account".into(),
            vec![ReviewInput {
                label: "title".into(),
                value: "Fixture".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(store.pending().await.unwrap().len(), 1);
    assert_eq!(
        store
            .review(&review.id)
            .await
            .unwrap()
            .request
            .unwrap()
            .inputs,
        req.inputs
    );

    // An approval the user has not answered cannot be used, and stays.
    let waiting = store.consume(&review.id, &req).await.unwrap_err();
    assert_eq!(waiting.public_code, "approval_pending");
    assert_eq!(waiting.extra["approval"], review.id.as_str());
    assert_eq!(
        waiting.extra["command"],
        format!("itda approve {}", review.id)
    );
    assert!(waiting.extra["expires_at"].as_str().unwrap().contains('T'));
    assert_eq!(store.pending().await.unwrap().len(), 1);
    store.claim(&review.id).await.unwrap();
    assert_eq!(store.review(&review.id).await.unwrap().status, "approved");
    store.consume(&review.id, &req).await.unwrap();
    // Second use is refused.
    assert_eq!(
        store
            .consume(&review.id, &req)
            .await
            .unwrap_err()
            .public_code,
        "approval_expired"
    );
    assert!(store.pending().await.unwrap().is_empty());
}

#[tokio::test]
async fn changed_inputs_withdraw_a_granted_approval() {
    let store = approvals();
    let req = request(&[("title", "Fixture")]);
    let review = store
        .request_approval(req.clone(), "S".into(), "A".into(), "U".into(), vec![])
        .await
        .unwrap();
    store.claim(&review.id).await.unwrap();
    let withdrawn = store
        .consume(&review.id, &request(&[("title", "Other")]))
        .await
        .unwrap_err();
    assert_eq!(withdrawn.public_code, "approval_expired");
    assert_eq!(withdrawn.extra["reason"], "inputs_changed");
    // Not even the exact request works now.
    let gone = store.consume(&review.id, &req).await.unwrap_err();
    assert_eq!(gone.public_code, "approval_expired");
    assert!(gone.extra.get("reason").is_none());
    assert!(store.review(&review.id).await.is_err());
}

#[tokio::test]
async fn a_granted_approval_can_still_be_rejected_and_invalidation_is_exact() {
    let store = approvals();
    let req = request(&[]);
    let granted = store
        .request_approval(req.clone(), "S".into(), "A".into(), "U".into(), vec![])
        .await
        .unwrap();
    store.claim(&granted.id).await.unwrap();
    store.reject(&granted.id).await.unwrap();
    assert_eq!(
        store
            .consume(&granted.id, &req)
            .await
            .unwrap_err()
            .public_code,
        "approval_expired"
    );

    // The same site, another account: only the named account's approvals go.
    let mut other = req.clone();
    other.account_id = "other".into();
    let mine = store
        .request_approval(req.clone(), "S".into(), "A".into(), "U".into(), vec![])
        .await
        .unwrap();
    let theirs = store
        .request_approval(other.clone(), "S".into(), "A".into(), "U".into(), vec![])
        .await
        .unwrap();
    store
        .invalidate(Some("fixture"), Some("acct"))
        .await
        .unwrap();
    assert!(store.review(&mine.id).await.is_err());
    assert!(store.review(&theirs.id).await.is_ok());
    // With nothing named, nothing changes.
    store.invalidate(None, None).await.unwrap();
    assert!(store.review(&theirs.id).await.is_ok());
    store.invalidate(None, Some("other")).await.unwrap();
    assert!(store.review(&theirs.id).await.is_err());
}

#[tokio::test]
async fn rejected_and_invalidated_approvals_cannot_be_used() {
    let store = approvals();
    let req = request(&[]);
    let first = store
        .request_approval(req.clone(), "S".into(), "A".into(), "U".into(), vec![])
        .await
        .unwrap();
    store.reject(&first.id).await.unwrap();
    assert!(store.consume(&first.id, &req).await.is_err());

    let second = store
        .request_approval(req.clone(), "S".into(), "A".into(), "U".into(), vec![])
        .await
        .unwrap();
    store.invalidate(Some("fixture"), None).await.unwrap();
    assert!(store.claim(&second.id).await.is_err());
    assert!(store.consume(&second.id, &req).await.is_err());
}

#[test]
fn valid_fixture_passes_validation() {
    validate_spec(&fixture("https://example.com")).unwrap();
}

#[test]
fn unsafe_specs_are_refused() {
    let base = json!({"spec_version":1,"site":{"name":"fixture","base_url":"https://example.com"},"operations":[{"name":"read-items","method":"GET","path":"/items","effect":"read","evidence":"doc"}]});
    let mutate = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut value = base.clone();
        f(&mut value);
        spec_from(value)
    };
    let cases: Vec<(&str, Spec)> = vec![
        (
            "base url with query",
            mutate(&|v| v["site"]["base_url"] = json!("https://example.com/?a=1")),
        ),
        (
            "embedded credentials",
            mutate(&|v| v["site"]["base_url"] = json!("https://u:p@example.com")),
        ),
        (
            "secret default",
            mutate(&|v| {
                v["operations"][0]["query"] =
                    json!({"q":{"type":"string","default":"Bearer abcdefghijklmnop"}})
            }),
        ),
        (
            "credential-named input",
            mutate(&|v| v["operations"][0]["query"] = json!({"api_key":{"type":"string"}})),
        ),
        (
            "header auth from another site",
            mutate(&|v| {
                v["auth"] = json!([{"name":"key","kind":"header","header":"X-Key","value_from":"store:other/key"}]);
                v["operations"][0]["auth"] = json!("key");
            }),
        ),
        (
            "header auth to another origin",
            mutate(&|v| {
                v["auth"] = json!([{"name":"key","kind":"header","header":"X-Key","value_from":"store:fixture/key"}]);
                v["operations"][0]["auth"] = json!("key");
                v["operations"][0]["base_url"] = json!("https://api.example.com");
            }),
        ),
        (
            "derived credentials have no interpreter",
            mutate(&|v| {
                v["auth"] = json!([{"name":"d","kind":"derived","header":"X-CSRF-Token","value_from":"cookie:ct0"}]);
                v["operations"][0]["auth"] = json!("d");
            }),
        ),
        (
            "browser transport",
            mutate(&|v| v["operations"][0]["transport"] = json!("browser")),
        ),
        (
            "browser-fetch transport",
            mutate(&|v| v["operations"][0]["transport"] = json!("browser-fetch")),
        ),
        (
            "method override header",
            mutate(&|v| v["operations"][0]["headers"] = json!({"X-HTTP-Method-Override":"DELETE"})),
        ),
        (
            "method override input",
            mutate(&|v| v["operations"][0]["query"] = json!({"_method":{"type":"string"}})),
        ),
        (
            "browser imitation header",
            mutate(&|v| v["defaults"] = json!({"headers":{"Sec-Fetch-Mode":"navigate"}})),
        ),
        (
            "invalid html selector",
            mutate(&|v| {
                v["operations"][0]["response"] =
                    json!({"format":"html","required_html_fields":[{"selector":"div[["}]})
            }),
        ),
        (
            "header injection",
            mutate(&|v| v["operations"][0]["headers"] = json!({"X-A":"1\r\nHost: evil"})),
        ),
    ];
    for (name, sp) in cases {
        assert!(validate_spec(&sp).is_err(), "{name}");
    }
}

#[test]
fn refusals_name_every_field_and_keep_their_codes() {
    let sp = spec_from(
        json!({"spec_version":1,"site":{"name":"fixture","base_url":"https://example.com"},"auth":[{"name":"key","kind":"header","header":"X-Key","value_from":"store:fixture/key"}],"operations":[
            {"name":"read-items","method":"GET","path":"/items","effect":"read","evidence":"doc","headers":{"X-A":"1\r\nHost: evil"}},
            {"name":"other-host","method":"GET","path":"/x","effect":"read","evidence":"doc","auth":"key","base_url":"https://api.example.com"}
        ]}),
    );
    let problems = spec_problems(&sp);
    let fields: Vec<&str> = problems.iter().map(|p| p.field.as_str()).collect();
    assert!(
        fields.contains(&"operations.read-items.headers.X-A"),
        "{fields:?}"
    );
    assert!(fields.contains(&"operations.other-host.auth"), "{fields:?}");

    let error = validate_spec(&sp).unwrap_err();
    assert_eq!(error.public_code, problems[0].code);
    assert!(error.msg.starts_with(&problems[0].field), "{}", error.msg);

    let unsafe_auth = problems
        .iter()
        .find(|p| p.field == "operations.other-host.auth")
        .unwrap();
    assert_eq!(unsafe_auth.code, "unsafe_target");
}

#[test]
fn a_site_interval_must_be_a_time() {
    let mut sp = fixture("https://example.test");
    sp.defaults.interval = "soon".into();
    assert!(
        spec_problems(&sp)
            .iter()
            .any(|p| p.field == "defaults.interval")
    );
    sp.defaults.interval = "2s".into();
    assert!(
        spec_problems(&sp)
            .iter()
            .all(|p| p.field != "defaults.interval")
    );
}

#[test]
fn a_document_nested_too_deep_is_refused_without_being_parsed() {
    let deep = format!("query {}{}", "{ a ".repeat(40), "}".repeat(40));
    let mut op = fixture("https://example.com").operations.remove(0);
    op.method = "POST".into();
    op.path = "/graphql".into();
    let mut args = BTreeMap::new();
    args.insert("query".to_owned(), deep.clone());
    assert_eq!(operation_effect(&op, &args), Effect::Unknown);

    let spec = spec_from(
        json!({"spec_version":1,"site":{"name":"g","base_url":"https://g.example"},
        "operations":[{"name":"notices","method":"POST","path":"/graphql","effect":"read","evidence":"doc",
            "body":{"json":{"query":{"type":"string","default":deep}}}}]}),
    );
    let error = validate_spec(&spec).unwrap_err();
    assert!(
        error.msg.contains("nests deeper than 32 levels"),
        "{}",
        error.msg
    );
    let shallow = format!("query {}{}", "{ a ".repeat(30), "}".repeat(30));
    let spec = spec_from(
        json!({"spec_version":1,"site":{"name":"g","base_url":"https://g.example"},
        "operations":[{"name":"notices","method":"POST","path":"/graphql","effect":"read","evidence":"doc",
            "body":{"json":{"query":{"type":"string","default":shallow}}}}]}),
    );
    validate_spec(&spec).unwrap();
}

#[test]
fn pagination_is_for_reads_and_may_name_a_page_token() {
    let base = json!({"spec_version":1,"site":{"name":"p","base_url":"https://p.example"}});
    let with = |op: serde_json::Value| {
        let mut value = base.clone();
        value["operations"] = json!([op]);
        spec_from(value)
    };
    validate_spec(&with(
        json!({"name":"list","method":"GET","path":"/items","effect":"read","evidence":"doc",
        "query":{"page_token":{"type":"string"}},
        "pagination":{"style":"cursor","next":"/next_page_token","send":{"query":"page_token"}}}),
    ))
    .unwrap();
    let error = validate_spec(&with(
        json!({"name":"create","method":"POST","path":"/items","effect":"write","evidence":"doc",
        "body":{"json":{"title":{"type":"string"},"cursor":{"type":"string"}}},
        "pagination":{"style":"cursor","next":"/next","send":{"body":"cursor"}}}),
    ))
    .unwrap_err();
    assert!(
        error.msg.contains("only a read turns pages"),
        "{}",
        error.msg
    );
    let error = validate_spec(&with(
        json!({"name":"list","method":"GET","path":"/items","effect":"read","evidence":"doc",
        "query":{"cursor":{"type":"string"}},
        "pagination":{"style":"cursor","next":"/session_token","send":{"query":"cursor"}}}),
    ))
    .unwrap_err();
    assert!(error.msg.contains("credential-named"), "{}", error.msg);
    let error = validate_spec(&with(
        json!({"name":"list","method":"GET","path":"/items","effect":"read","evidence":"doc",
        "query":{"auth_token":{"type":"string"}},
        "pagination":{"style":"cursor","next":"/next","send":{"query":"auth_token"}}}),
    ))
    .unwrap_err();
    assert!(
        error.msg.contains("credentials must come from"),
        "{}",
        error.msg
    );
}

#[test]
fn a_confirming_read_must_really_read() {
    let base = json!({"spec_version":1,"site":{"name":"p","base_url":"https://p.example"}});
    let with = |ops: serde_json::Value| {
        let mut value = base.clone();
        value["operations"] = ops;
        spec_from(value)
    };
    let create = |read: &str| {
        json!({"name":"create","method":"POST","path":"/items","effect":"write","evidence":"doc",
        "body":{"json":{"title":{"type":"string","required":true}}},
        "confirm":{"read":read,"match":{"pointer":"/items/*/title","from":"body.title"}}})
    };
    validate_spec(&with(json!([
        {"name":"list","method":"GET","path":"/items","effect":"read","evidence":"doc"},
        create("list")
    ])))
    .unwrap();
    // `effect: read` on a GET /items/delete is still a change for the
    // classifier, so it cannot confirm anything.
    let error = validate_spec(&with(json!([
        {"name":"clear","method":"GET","path":"/items/delete","effect":"read","evidence":"doc"},
        create("clear")
    ])))
    .unwrap_err();
    assert!(
        error.msg.contains("clear would change the site"),
        "{}",
        error.msg
    );
    assert!(
        error.msg.contains("operations.create.confirm.read"),
        "{}",
        error.msg
    );
    // A read has no outcome to check.
    let error = validate_spec(&with(json!([
        {"name":"list","method":"GET","path":"/items","effect":"read","evidence":"doc"},
        {"name":"peek","method":"GET","path":"/peek","effect":"read","evidence":"doc",
         "query":{"title":{"type":"string","required":true}},
         "confirm":{"read":"list","match":{"pointer":"/items/*/title","from":"query.title"}}}
    ])))
    .unwrap_err();
    assert!(
        error
            .msg
            .contains("only a change names the read that confirms it"),
        "{}",
        error.msg
    );
}

#[test]
fn a_sign_in_form_belongs_to_a_browser_credential_on_the_site() {
    let with = |kind: &str, path: &str, user: &str, password: &str| {
        spec_from(
            json!({"spec_version":1,"site":{"name":"board","base_url":"https://board.example"},
            "auth":[{"name":"session","kind":kind,"value_from":"cookie:SID",
                     "login":{"path":path,"user":user,"password":password}}],
            "operations":[{"name":"list","method":"GET","path":"/list","auth":"session","effect":"read","evidence":"doc"}]}),
        )
    };
    validate_spec(&with("browser", "/login", "user_name", "password")).unwrap();
    validate_spec(&with(
        "browser",
        "/index.php?title=Special:UserLogin",
        "wpName",
        "wpPassword",
    ))
    .unwrap();
    for (kind, path, user, password) in [
        ("header", "/login", "user", "password"),
        ("browser", "login", "user", "password"),
        ("browser", "//evil.example/login", "user", "password"),
        ("browser", "/login#x", "user", "password"),
        ("browser", "/login", "", "password"),
        ("browser", "/login", "same", "same"),
        ("browser", "/login", "a&b", "password"),
    ] {
        let problems = spec_problems(&with(kind, path, user, password));
        assert!(
            problems
                .iter()
                .any(|p| p.field.starts_with("auth.session.login")),
            "{kind} {path} {user} {password}: {problems:?}"
        );
    }
}

#[test]
fn a_redirect_or_page_address_carrying_a_graphql_mutation_is_not_a_safe_read() {
    for raw in [
        "https://example.com/graphql?query=mutation%20%7B%20deleteItem(id%3A%201)%20%7D",
        "https://example.com/graphql?query=mutation{deleteX}",
        "https://example.com/api?query=subscription%20%7B%20updates%20%7D",
        "https://example.com/graphql?q=mutation%20%7B%20x%20%7D",
        "https://example.com/graphql?document=mutation%20%7B%20x%20%7D",
        "https://example.com/graphql?query=mutation%20%7B%20broken",
        // A document itda cannot read is not followed either, as the
        // declared classifier treats the same value.
        "https://example.com/search?query=mutation%20testing%20books",
    ] {
        assert!(!safe_read_url(&url::Url::parse(raw).unwrap()), "{raw}");
    }
    for raw in [
        "https://example.com/graphql?query=%7B%20items%20%7B%20id%20%7D%20%7D",
        "https://example.com/graphql?query=query%20Items%20%7B%20items%20%7B%20id%20%7D%20%7D",
        "https://example.com/search?query=books%20about%20mutation",
        "https://example.com/search?q=mutation%20%7B%20x%20%7D",
        "https://example.com/graphql?query=%23%20mutation%20is%20a%20comment%0A%7B%20items%20%7B%20id%20%7D%20%7D",
    ] {
        assert!(safe_read_url(&url::Url::parse(raw).unwrap()), "{raw}");
    }
}

#[test]
fn the_rendered_request_is_classified_from_what_is_sent() {
    use itda_policy::rendered_request_effect;
    let url = |raw: &str| url::Url::parse(raw).unwrap();
    let json = |value: serde_json::Value| Some(serde_json::to_vec(&value).unwrap());

    // A literal mutation in a json_template, or a nested intent field,
    // turns a declared read into a change.
    let mutation = rendered_request_effect(
        &url("https://example.com/graphql"),
        json(json!({"query": "mutation { deleteItem(id: 1) { id } }", "variables": {}})).as_deref(),
        false,
    );
    assert_eq!(mutation.effect, Effect::Write);
    assert_eq!(mutation.reason, "body names a mutation");
    let nested = rendered_request_effect(
        &url("https://example.com/search"),
        json(json!({"filter": {"items": [{"settings": {"op": "delete"}}]}})).as_deref(),
        false,
    );
    assert_eq!(nested.effect, Effect::Write);
    assert_eq!(nested.reason, "body asks for a change");
    let key = rendered_request_effect(
        &url("https://example.com/search"),
        json(json!({"deleteAll": true, "q": "books"})).as_deref(),
        false,
    );
    assert_eq!(key.effect, Effect::Write);
    let form = rendered_request_effect(
        &url("https://example.com/search"),
        Some(b"q=books&mode=delete"),
        true,
    );
    assert_eq!(form.effect, Effect::Write);
    let subscription = rendered_request_effect(
        &url("https://example.com/search"),
        Some(b"q=books&query=subscription+%7B+updates+%7D"),
        true,
    );
    assert_eq!(subscription.effect, Effect::Write);
    let address = rendered_request_effect(
        &url("https://example.com/graphql?query=mutation%20%7B%20x%20%7D"),
        None,
        false,
    );
    assert_eq!(address.effect, Effect::Write);
    assert_eq!(address.reason, "address names a mutation");
    let address_key =
        rendered_request_effect(&url("https://example.com/items?purge=1"), None, false);
    assert_eq!(address_key.effect, Effect::Write);

    // A GraphQL endpoint with no document, or a document itda cannot read,
    // is unknown and carries a remedy.
    let bare = rendered_request_effect(
        &url("https://example.com/graphql"),
        json(json!({"variables": {}})).as_deref(),
        false,
    );
    assert_eq!(bare.effect, Effect::Unknown);
    assert_eq!(bare.reason, "the body names no GraphQL document");
    let broken = rendered_request_effect(
        &url("https://example.com/graphql"),
        json(json!({"query": "mutation { broken"})).as_deref(),
        false,
    );
    assert_eq!(broken.effect, Effect::Unknown);
    let deep = format!("query {}{}", "{ a ".repeat(40), "}".repeat(40));
    let too_deep = rendered_request_effect(
        &url("https://example.com/graphql"),
        json(json!({"query": deep})).as_deref(),
        false,
    );
    assert_eq!(too_deep.effect, Effect::Unknown);
    let nested_too_deep = {
        let mut value = json!({"q": "books"});
        for _ in 0..40 {
            value = json!({"inner": value});
        }
        rendered_request_effect(
            &url("https://example.com/search"),
            json(value).as_deref(),
            false,
        )
    };
    assert_eq!(nested_too_deep.effect, Effect::Unknown);

    // Reads stay reads: a GraphQL query, a search whose text names a change,
    // a value that only mentions a mutation, and a plain page.
    for (address, body, form) in [
        (
            "https://example.com/graphql",
            json(json!({"query": "query Items { items { id } }", "variables": {"first": 10}})),
            false,
        ),
        (
            "https://example.com/graphql",
            json(json!({"query": "{ items(search: \"mutation\") { id } }"})),
            false,
        ),
        (
            "https://example.com/graphql",
            json(json!({"query": "# mutation is a comment\n{ items { id } }"})),
            false,
        ),
        (
            "https://example.com/search",
            json(json!({"q": "how to delete an account", "page": 2, "sort": "updated"})),
            false,
        ),
        (
            "https://example.com/search?q=remove+old+posts&created_after=2020",
            None,
            false,
        ),
        (
            "https://example.com/search",
            Some(b"q=delete+me&page=2".to_vec()),
            true,
        ),
        ("https://example.com/items?page=2", None, false),
        (
            "https://example.com/search",
            json(json!({"filter": {"settings": {"mode": "fast"}}})),
            false,
        ),
    ] {
        let verdict = rendered_request_effect(&url(address), body.as_deref(), form);
        assert_eq!(verdict.effect, Effect::Read, "{address} {body:?}");
        assert_eq!(verdict.reason, "", "{address}");
    }
}

#[test]
fn url_rewrite_headers_override_the_request_like_method_overrides() {
    for name in [
        "X-Original-URL",
        "x-rewrite-url",
        "X-HTTP-Method-Override",
        "_method",
    ] {
        assert!(itda_policy::overrides_method(name), "{name}");
    }
    assert!(!itda_policy::overrides_method("X-Requested-With"));
    let spec = spec_from(
        json!({"spec_version":1,"site":{"name":"fixture","base_url":"https://example.com"},
        "operations":[{"name":"read-items","method":"GET","path":"/items","effect":"read","evidence":"doc",
            "headers":{"X-Original-URL":"/admin/delete"}}]}),
    );
    let problems = spec_problems(&spec);
    assert!(
        problems
            .iter()
            .any(|p| p.field == "operations.read-items.headers.X-Original-URL"),
        "{problems:?}"
    );
}

/// Every read in the bundled definitions, rendered with its defaults (and a
/// placeholder for each required input), is still a read when the request
/// itself is read: the gate re-reading the request adds no approval to a
/// correctly declared definition.
#[test]
fn bundled_reads_stay_reads_when_the_rendered_request_is_read() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sites");
    let mut files: Vec<_> = ["", "software"]
        .iter()
        .flat_map(|dir| std::fs::read_dir(root.join(dir)).unwrap())
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 12, "{files:?}");
    let (mut reads, mut changes) = (0, 0);
    for file in &files {
        let spec = spec::parse(&std::fs::read(file).unwrap()).unwrap();
        for op in &spec.operations {
            let mut args = BTreeMap::new();
            for (name, param) in op
                .params
                .iter()
                .chain(op.query.iter())
                .chain(op.body.iter().flat_map(|body| body.inputs()))
            {
                if param.required && param.default.is_none() {
                    let value = match param.kind.as_str() {
                        "int" | "float" => "1",
                        "bool" => "true",
                        _ => "x",
                    };
                    args.insert(name.clone(), value.to_owned());
                }
            }
            if operation_effect(op, &args) != Effect::Read {
                changes += 1;
                continue;
            }
            reads += 1;
            let (url, body) = spec.build_request(op, &args).unwrap();
            let url = url::Url::parse(&url).unwrap();
            let form = op.body.as_ref().is_some_and(|body| !body.form.is_empty());
            let rendered = itda_policy::rendered_request_effect(&url, body.as_deref(), form);
            assert_eq!(
                rendered.effect,
                Effect::Read,
                "{}: {} ({})",
                file.display(),
                op.name,
                rendered.reason
            );
            assert!(safe_read_url(&url), "{}: {}", file.display(), op.name);
        }
    }
    eprintln!("bundled definitions: {reads} reads stay reads, {changes} changes untouched");
    assert!(reads > 50, "{reads}");
}

#[test]
fn a_body_that_carries_a_file_is_always_a_change() {
    let sp = spec_from(
        json!({"spec_version":1,"site":{"name":"fixture","base_url":"https://example.com"},"operations":[
            {"name":"search-files","method":"GET","path":"/search","effect":"read","evidence":"doc",
             "body":{"multipart":{"fields":{"title":{"type":"string"}},"file":{"name":"upload","from":"attachment"}}}}
        ]}),
    );
    let op = &sp.operations[0];
    assert_eq!(operation_effect(op, &BTreeMap::new()), Effect::Write);
    let names: Vec<&str> = op
        .body
        .as_ref()
        .unwrap()
        .inputs()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(names, ["title", "attachment"]);
    assert_eq!(op.file_input().map(String::as_str), Some("attachment"));
}

#[test]
fn file_reads_and_uploads_are_checked_before_install() {
    let base = json!({"spec_version":1,"site":{"name":"fixture","base_url":"https://example.com"},"operations":[
        {"name":"get-attachment","method":"GET","path":"/files/{id}","effect":"read","evidence":"doc",
         "params":{"id":{"type":"int"}},
         "response":{"format":"file","media_types":["application/pdf"]}},
        {"name":"upload-file","method":"POST","path":"/files","effect":"write","evidence":"doc",
         "body":{"multipart":{"fields":{"title":{"type":"string"}},"file":{"name":"upload","from":"attachment"}}}}
    ]});
    validate_spec(&spec_from(base.clone())).unwrap();
    let refused = |what: &str, f: &dyn Fn(&mut serde_json::Value), code: &str, field: &str| {
        let mut value = base.clone();
        f(&mut value);
        let problems = spec_problems(&spec_from(value));
        let found = problems.iter().find(|p| p.field == field);
        assert!(found.is_some(), "{what}: {problems:?}");
        assert_eq!(found.unwrap().code, code, "{what}");
    };
    refused(
        "no media types",
        &|v| v["operations"][0]["response"] = json!({"format": "file"}),
        "data_invalid",
        "operations.get-attachment.response.media_types",
    );
    refused(
        "a model on a file",
        &|v| {
            v["models"] = json!({"m": {"id": {"type": "int"}}});
            v["operations"][0]["model"] = json!("m");
        },
        "data_invalid",
        "operations.get-attachment.response.format",
    );
    refused(
        "pointers on a file",
        &|v| v["operations"][0]["response"]["required_pointers"] = json!(["/a"]),
        "data_invalid",
        "operations.get-attachment.response",
    );
    refused(
        "media types on a json read",
        &|v| {
            v["operations"][0]["response"] =
                json!({"format": "json", "media_types": ["application/pdf"]})
        },
        "data_invalid",
        "operations.get-attachment.response",
    );
    refused(
        "a file from a change",
        &|v| v["operations"][0]["method"] = json!("DELETE"),
        "data_invalid",
        "operations.get-attachment.response.format",
    );
    refused(
        "a cap past what itda holds",
        &|v| v["operations"][0]["response"]["max_bytes"] = json!(2_u64 * 1024 * 1024 * 1024),
        "data_invalid",
        "operations.get-attachment.response.max_bytes",
    );
    refused(
        "a file to another origin",
        &|v| v["operations"][1]["base_url"] = json!("https://uploads.example.com"),
        "unsafe_target",
        "operations.upload-file.body.multipart",
    );
    refused(
        "a credential-named file input",
        &|v| v["operations"][1]["body"]["multipart"]["file"]["from"] = json!("token"),
        "data_invalid",
        "operations.upload-file.body.multipart.file.from",
    );
    refused(
        "a file input that is also a text field",
        &|v| {
            v["operations"][1]["body"]["multipart"]["fields"]["attachment"] =
                json!({"type": "string"})
        },
        "data_invalid",
        "operations.upload-file.body.multipart.file.from",
    );
}
