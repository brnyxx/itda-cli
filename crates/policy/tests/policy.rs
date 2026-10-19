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

    // A different input does not match the approved digest.
    assert!(
        store
            .consume(&review.id, &request(&[("title", "Other")]))
            .await
            .is_err()
    );
    store.claim(&review.id).await.unwrap();
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
            "derived auth sets a forbidden header",
            mutate(&|v| {
                v["auth"] = json!([{"name":"d","kind":"derived","header":"Cookie","value_from":"cookie:sid"}]);
                v["operations"][0]["auth"] = json!("d");
            }),
        ),
        (
            "rendered browser read that is not html",
            mutate(&|v| v["operations"][0]["transport"] = json!("browser")),
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
