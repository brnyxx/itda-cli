//! Classification cases are derived from Hycli `src/integration_tests.rs`
//! (Apache-2.0); approval and validation cases are new.
#![allow(clippy::unwrap_used)]
use itda_policy::{Effect, operation_effect, safe_read_url};
use itda_spec::{self as spec, Spec};
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
