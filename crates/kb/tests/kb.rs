//! The knowledge base on an in-memory store and on a database another
//! component already wrote. Nothing here touches the network.
#![allow(clippy::unwrap_used)]
use itda_kb::{ATTEMPTS_PER_ACTION, Attempt, Declared, Kb, Ref, Stage};
use itda_store::{Migration, Store};
use serde_json::json;

fn kb() -> Kb {
    Kb::open(Store::in_memory().unwrap()).unwrap()
}

fn action(kb_: &Kb) -> Ref<'static> {
    let _ = kb_;
    Ref {
        kind: "action",
        key: "list-items",
    }
}

async fn tried(kb_: &Kb, action: &str, verdicts: &[&str]) {
    for verdict in verdicts {
        kb_.attempt(Attempt {
            site: "s".into(),
            action: action.into(),
            stage: Stage::Check,
            verdict: (*verdict).into(),
            http_status: Some(if *verdict == "ok" { 200 } else { 500 }),
            duration_ms: Some(12),
            ..Attempt::default()
        })
        .await
        .unwrap();
    }
}

/// Calls itda refused before sending: no HTTP status.
async fn refused(kb_: &Kb, action: &str, verdict: &str, n: usize) {
    for _ in 0..n {
        kb_.attempt(Attempt {
            site: "s".into(),
            action: action.into(),
            verdict: verdict.into(),
            ..Attempt::default()
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn fts5_is_in_this_build_and_finds_a_finding() {
    let kb_ = kb();
    kb_.finding(
        "s",
        action(&kb_),
        "kept: answered with 12 records",
        "setup",
        1.0,
    )
    .await
    .unwrap();
    kb_.finding(
        "s",
        action(&kb_),
        "removed: response_mismatch",
        "setup",
        1.0,
    )
    .await
    .unwrap();
    let found = kb_.search(Some("s"), "records").await.unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].statement.contains("12 records"));
    assert!(
        kb_.search(Some("other"), "records")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(kb_.search(None, "mismatch").await.unwrap().len(), 1);
    // Query words are words, never FTS operators.
    assert!(kb_.search(None, "mismatch OR").await.unwrap().is_empty());
    assert!(kb_.search(None, "").await.unwrap().is_empty());
}

#[tokio::test]
async fn the_schema_opens_a_database_another_component_wrote() {
    static OLDER: Migration = Migration {
        component: "cursors",
        steps: &["CREATE TABLE cursors(handle TEXT PRIMARY KEY, binding TEXT NOT NULL)"],
    };
    let dir = std::env::temp_dir().join(format!("itda-kb-{}", itda_types::util::id()));
    let path = dir.join("itda.db");
    {
        let store = Store::open(&path).unwrap();
        store.migrate(&OLDER).unwrap();
        store
            .call_blocking(|conn| {
                conn.execute("INSERT INTO cursors VALUES('ic_1','b')", [])?;
                Ok(())
            })
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let kb_ = Kb::open(store.clone()).unwrap();
    kb_.node("s", "page", "/", json!({})).await.unwrap();
    let kept: i64 = store
        .call_blocking(|conn| Ok(conn.query_row("SELECT count(*) FROM cursors", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(kept, 1);
    // Opening again runs no step twice.
    Kb::open(store).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn nodes_merge_props_and_edges_need_both_ends() {
    let kb_ = kb();
    let a = kb_
        .node("s", "action", "a", json!({"method": "GET"}))
        .await
        .unwrap();
    let again = kb_
        .node("s", "action", "a", json!({"effect": "read"}))
        .await
        .unwrap();
    assert_eq!(a, again);
    let nodes = kb_.nodes("s", Some("action"), None).await.unwrap();
    assert_eq!(nodes[0].props, json!({"method": "GET", "effect": "read"}));
    let page = Ref {
        kind: "page",
        key: "/",
    };
    let act = Ref {
        kind: "action",
        key: "a",
    };
    assert!(
        kb_.edge("s", page, act, "drafted_from", json!({}))
            .await
            .is_err()
    );
    kb_.node("s", "page", "/", json!({})).await.unwrap();
    kb_.edge("s", act, page, "drafted_from", json!({}))
        .await
        .unwrap();
    assert!(kb_.edge("s", act, page, "likes", json!({})).await.is_err());
    assert!(kb_.node("s", "thing", "x", json!({})).await.is_err());
    assert_eq!(kb_.edges("s").await.unwrap().len(), 1);
}

#[tokio::test]
async fn attempts_are_capped_per_action_and_feed_the_stats() {
    let kb_ = kb();
    let verdicts: Vec<&str> = (0..ATTEMPTS_PER_ACTION + 5)
        .map(|i| {
            if i % 3 == 0 {
                "response_mismatch"
            } else {
                "ok"
            }
        })
        .collect();
    tried(&kb_, "list-items", &verdicts).await;
    let counts = kb_.counts("s").await.unwrap();
    assert_eq!(counts.attempts, ATTEMPTS_PER_ACTION);
    let stats = kb_.stats("s").await.unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].attempts, itda_kb::RECENT);
    assert!(stats[0].pass_rate > 0.5 && stats[0].pass_rate < 1.0);
    assert!(stats[0].last_ok_at.is_some());
    // Six ok in a row: verified and stable.
    tried(&kb_, "list-items", &["ok"; 7]).await;
    let stats = kb_.stats("s").await.unwrap();
    assert!(stats[0].verified);
    assert!(!stats[0].unstable);
    let summary = itda_kb::summary(&stats);
    assert_eq!(summary.verified, 1);
    assert!(summary.pass_rate.unwrap() > 0.5);
    assert!(summary.unstable.is_empty());
    assert_eq!(itda_kb::summary(&[]).pass_rate, None);
}

#[tokio::test]
async fn the_shape_of_an_answer_is_kept_on_the_action() {
    let kb_ = kb();
    kb_.attempt(Attempt {
        site: "s".into(),
        action: "list-items".into(),
        verdict: "ok".into(),
        shape: Some(json!({"items": [{"id": "integer", "title": "string"}]})),
        ..Attempt::default()
    })
    .await
    .unwrap();
    let node = &kb_
        .nodes("s", Some("action"), Some("list-items"))
        .await
        .unwrap()[0];
    assert_eq!(node.props["shape"]["items"][0]["title"], "string");
}

#[tokio::test]
async fn gap_endpoint_untried_names_a_read_nobody_ran() {
    let kb_ = kb();
    kb_.node(
        "s",
        "action",
        "list-items",
        Declared {
            method: "GET",
            path: "/items",
            effect: "read",
            desc: "List items",
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].rule, "endpoint_untried");
    assert_eq!(gaps[0].subject, "list-items");
    assert_eq!(gaps[0].remedy.command, "itda check s");
    assert_eq!(gaps[0].remedy.tool, "itda_check");
    assert_eq!(gaps[0].remedy.arguments, json!({"site": "s"}));
    assert_eq!(kb_.counts("s").await.unwrap().gaps, 1);
    tried(&kb_, "list-items", &["ok"]).await;
    assert!(kb_.gaps("s", Vec::new()).await.unwrap().is_empty());
    // A check skips a read that saves a file: it is called by name.
    kb_.node(
        "s",
        "action",
        "get-file",
        Declared {
            method: "GET",
            path: "/f",
            effect: "read",
            saves_file: true,
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    assert_eq!(gaps[0].remedy.command, "itda describe s.get-file");
}

#[tokio::test]
async fn gap_read_needs_input_says_which_list_read_can_supply_it() {
    let kb_ = kb();
    kb_.node(
        "s",
        "action",
        "list-items",
        Declared {
            method: "GET",
            path: "/items",
            effect: "read",
            desc: "List items",
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    tried(&kb_, "list-items", &["ok"]).await;
    kb_.node(
        "s",
        "action",
        "get-item",
        Declared {
            method: "GET",
            path: "/items/{id}",
            effect: "read",
            desc: "Show one item",
            needs: &["id".into()],
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    let needs: Vec<&itda_kb::Gap> = gaps
        .iter()
        .filter(|g| g.rule == "read_needs_input")
        .collect();
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].reason, "get-item needs id");
    assert_eq!(needs[0].remedy.command, "itda describe s.get-item");
    assert_eq!(needs[0].remedy.tool, "itda_sites");
    kb_.edge(
        "s",
        Ref {
            kind: "action",
            key: "list-items",
        },
        Ref {
            kind: "action",
            key: "get-item",
        },
        "feeds_input",
        json!({"pointer": "/items/0/id", "input": "id"}),
    )
    .await
    .unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    let fed = gaps.iter().find(|g| g.rule == "read_needs_input").unwrap();
    assert_eq!(
        fed.reason,
        "get-item needs id; list-items's /items/0/id can supply it"
    );
    assert_eq!(fed.remedy.command, "itda call s.list-items --json");
    assert_eq!(fed.remedy.tool, "s.list-items");
    assert_eq!(fed.remedy.arguments, json!({}));
    assert_eq!(fed.fields["pointer"], "/items/0/id");
    // Once it answered, the input is no longer a gap.
    tried(&kb_, "get-item", &["ok"]).await;
    assert!(
        kb_.gaps("s", Vec::new())
            .await
            .unwrap()
            .iter()
            .all(|g| g.rule != "read_needs_input")
    );
}

#[tokio::test]
async fn gap_finding_without_evidence_wants_a_try_behind_a_note() {
    let kb_ = kb();
    kb_.finding("s", action(&kb_), "kept: answered", "setup", 1.0)
        .await
        .unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    assert!(
        gaps.iter()
            .any(|g| g.rule == "finding_without_evidence" && g.subject == "list-items")
    );
    tried(&kb_, "list-items", &["ok"]).await;
    assert!(
        kb_.gaps("s", Vec::new())
            .await
            .unwrap()
            .iter()
            .all(|g| g.rule != "finding_without_evidence")
    );
}

#[tokio::test]
async fn gap_artifacts_notice_a_changed_or_missing_file() {
    let kb_ = kb();
    kb_.artifact("s", "spec", "specs/s/spec.yaml", "abc", 10)
        .await
        .unwrap();
    kb_.artifact("s", "check", "specs/s/check.json", "def", 10)
        .await
        .unwrap();
    assert_eq!(kb_.artifacts("s").await.unwrap().len(), 2);
    let same = vec![
        ("specs/s/spec.yaml".to_owned(), Some("abc".to_owned())),
        ("specs/s/check.json".to_owned(), Some("def".to_owned())),
    ];
    assert!(kb_.gaps("s", same).await.unwrap().is_empty());
    let now = vec![
        ("specs/s/spec.yaml".to_owned(), Some("zzz".to_owned())),
        ("specs/s/check.json".to_owned(), None),
    ];
    let gaps = kb_.gaps("s", now).await.unwrap();
    let rules: Vec<(&str, &str, u8)> = gaps
        .iter()
        .map(|g| (g.rule.as_str(), g.remedy.command.as_str(), g.priority))
        .collect();
    assert_eq!(
        rules,
        [
            ("artifact_changed", "itda check s", 1),
            ("artifact_missing", "itda check s", 1)
        ]
    );
    // A path not on the list is not judged.
    assert!(kb_.gaps("s", Vec::new()).await.unwrap().is_empty());
}

#[tokio::test]
async fn gap_flapping_read_fails_twice_in_the_last_seven() {
    let kb_ = kb();
    tried(
        &kb_,
        "list-items",
        &["ok", "ok", "timeout", "ok", "ok", "request_failed", "ok"],
    )
    .await;
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].rule, "flapping_read");
    assert_eq!(gaps[0].priority, 1);
    assert_eq!(gaps[0].reason, "list-items failed 2 of the last 7 tries");
    assert!(kb_.stats("s").await.unwrap()[0].unstable);
    // One failure is a blip, not flapping; all failures is broken, not flapping.
    let other = kb();
    tried(
        &other,
        "a",
        &["ok", "ok", "timeout", "ok", "ok", "ok", "ok"],
    )
    .await;
    tried(&other, "b", &["timeout", "timeout", "timeout"]).await;
    assert!(
        other
            .gaps("s", Vec::new())
            .await
            .unwrap()
            .iter()
            .all(|g| g.rule != "flapping_read")
    );
}

#[tokio::test]
async fn gap_dangling_edge_points_at_a_missing_node() {
    let kb_ = kb();
    let a = kb_.node("s", "action", "a", json!({})).await.unwrap();
    tried(&kb_, "a", &["ok"]).await;
    kb_.edge_ids(a, a + 999, "links_to").await.unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].rule, "dangling_edge");
    assert!(gaps[0].subject.starts_with("links_to:"));
    assert_eq!(gaps[0].remedy.command, "itda refresh s");
}

#[tokio::test]
async fn gaps_come_worst_first_and_forget_wipes_a_site() {
    let kb_ = kb();
    kb_.node(
        "s",
        "action",
        "never",
        Declared {
            method: "GET",
            path: "/n",
            effect: "read",
            desc: "",
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    tried(&kb_, "flap", &["ok", "x", "ok", "x", "ok"]).await;
    kb_.node("other", "page", "/", json!({})).await.unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    let rules: Vec<&str> = gaps.iter().map(|g| g.rule.as_str()).collect();
    assert_eq!(rules, ["flapping_read", "endpoint_untried"]);
    assert_eq!(kb_.sites().await.unwrap(), ["other", "s"]);
    kb_.forget("s").await.unwrap();
    assert_eq!(kb_.counts("s").await.unwrap(), itda_kb::Counts::default());
    assert_eq!(kb_.sites().await.unwrap(), ["other"]);
    assert!(kb_.dump().await.unwrap().contains("other"));
    assert!(!kb_.dump().await.unwrap().contains("flap"));
}

#[tokio::test]
async fn calls_refused_before_sending_are_not_tries() {
    let kb_ = kb();
    kb_.node(
        "s",
        "action",
        "post",
        Declared {
            method: "POST",
            path: "/p",
            effect: "write",
            desc: "Post",
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    refused(&kb_, "post", "approval_required", 1).await;
    refused(&kb_, "post", "approval_pending", 1).await;
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    // Never sent: a change nobody ran yet, after any read nobody ran.
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        (gaps[0].rule.as_str(), gaps[0].priority),
        ("endpoint_untried", 3)
    );
    assert_eq!(gaps[0].remedy.tool, "itda_sites");
    for _ in 0..2 {
        tried(&kb_, "post", &["ok"]).await;
        refused(&kb_, "post", "approval_required", 1).await;
    }
    tried(&kb_, "post", &["ok"]).await;
    let stats = kb_.stats("s").await.unwrap();
    assert_eq!(
        (stats[0].attempts, stats[0].passed, stats[0].unstable),
        (3, 3, false)
    );
    assert!(kb_.gaps("s", Vec::new()).await.unwrap().is_empty());
    // The record is still there.
    assert_eq!(kb_.counts("s").await.unwrap().attempts, 7);
}

#[tokio::test]
async fn a_preparation_loops_tries_and_notes_are_not_held_against_the_definition() {
    let kb_ = kb();
    kb_.node(
        "s",
        "action",
        "list",
        Declared {
            method: "GET",
            path: "/items",
            effect: "read",
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    for verdict in ["response_mismatch", "response_mismatch", "ok"] {
        kb_.attempt(Attempt {
            site: "s".into(),
            action: "list".into(),
            stage: Stage::Prepare,
            verdict: verdict.into(),
            http_status: Some(200),
            ..Attempt::default()
        })
        .await
        .unwrap();
    }
    // A draft's read the loop tried and dropped: a bare node, no gap.
    kb_.attempt(Attempt {
        site: "s".into(),
        action: "dropped-read".into(),
        stage: Stage::Prepare,
        verdict: "request_failed".into(),
        http_status: Some(404),
        ..Attempt::default()
    })
    .await
    .unwrap();
    kb_.finding(
        "s",
        Ref {
            kind: "endpoint",
            key: "GET /api/items",
        },
        "prepare: route in the docs (e3)",
        itda_kb::PREPARE_SCOPE,
        1.0,
    )
    .await
    .unwrap();
    let stats = kb_.stats("s").await.unwrap();
    let list = stats.iter().find(|s| s.action == "list").unwrap();
    assert_eq!((list.attempts, list.verified), (0, false));
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    let rules: Vec<(&str, &str)> = gaps
        .iter()
        .map(|g| (g.rule.as_str(), g.subject.as_str()))
        .collect();
    assert_eq!(rules, [("endpoint_untried", "list")]);
    // Still on record, and found by a search.
    assert_eq!(kb_.counts("s").await.unwrap().attempts, 4);
    assert_eq!(kb_.search(Some("s"), "docs").await.unwrap().len(), 1);
    // A try from a check counts as before.
    tried(&kb_, "list", &["ok"]).await;
    assert!(kb_.gaps("s", Vec::new()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_removed_action_leaves_the_stats_and_the_rules() {
    let kb_ = kb();
    tried(&kb_, "gone", &["ok", "timeout", "ok", "timeout"]).await;
    assert_eq!(kb_.gaps("s", Vec::new()).await.unwrap().len(), 1);
    kb_.node("s", "action", "gone", json!({"removed": true}))
        .await
        .unwrap();
    assert!(kb_.stats("s").await.unwrap().is_empty());
    assert!(kb_.gaps("s", Vec::new()).await.unwrap().is_empty());
    // Back in the definition, it counts again.
    kb_.node(
        "s",
        "action",
        "gone",
        Declared {
            method: "GET",
            path: "/g",
            effect: "read",
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    assert_eq!(kb_.stats("s").await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_read_that_needs_an_input_is_one_gap_not_three() {
    let kb_ = kb();
    kb_.node(
        "s",
        "action",
        "get-item",
        Declared {
            method: "GET",
            path: "/items/{id}",
            effect: "read",
            needs: &["id".into()],
            ..Declared::default()
        }
        .props(),
    )
    .await
    .unwrap();
    kb_.finding(
        "s",
        Ref {
            kind: "action",
            key: "get-item",
        },
        "skipped: needs id",
        "setup",
        1.0,
    )
    .await
    .unwrap();
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    let rules: Vec<&str> = gaps.iter().map(|g| g.rule.as_str()).collect();
    assert_eq!(rules, ["read_needs_input"]);
}

#[tokio::test]
async fn a_renamed_action_keeps_its_tries_and_leaves_the_stats() {
    let kb_ = kb();
    let a = Ref {
        kind: "action",
        key: "a",
    };
    tried(&kb_, "a", &["ok"]).await;
    assert_eq!(
        kb_.rename("s", a, "a@1234").await.unwrap().as_deref(),
        Some("a@1234")
    );
    assert_eq!(kb_.rename("s", a, "a@1234").await.unwrap(), None);
    assert!(kb_.stats("s").await.unwrap().is_empty());
    assert_eq!(kb_.counts("s").await.unwrap().attempts, 1);
    // The same old definition changed again: the history gets another name.
    tried(&kb_, "a", &["ok"]).await;
    assert_eq!(
        kb_.rename("s", a, "a@1234").await.unwrap().as_deref(),
        Some("a@1234.2")
    );
}

#[tokio::test]
async fn a_404_the_site_answered_is_a_try_and_a_read_that_keeps_failing_is_a_gap() {
    let kb_ = kb();
    for _ in 0..3 {
        kb_.attempt(Attempt {
            site: "s".into(),
            action: "gone".into(),
            stage: Stage::Check,
            verdict: "not_found".into(),
            http_status: Some(404),
            ..Attempt::default()
        })
        .await
        .unwrap();
    }
    let stats = kb_.stats("s").await.unwrap();
    assert_eq!((stats[0].attempts, stats[0].passed), (3, 0));
    let gaps = kb_.gaps("s", Vec::new()).await.unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        (gaps[0].rule.as_str(), gaps[0].priority),
        ("read_failing", 1)
    );
    assert_eq!(gaps[0].reason, "gone failed its last 3 tries");
    assert_eq!(gaps[0].remedy.command, "itda check s");
    // An answer after the failures: no longer failing, but flapping.
    tried(&kb_, "gone", &["ok"]).await;
    let rules: Vec<String> = kb_
        .gaps("s", Vec::new())
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.rule)
        .collect();
    assert_eq!(rules, ["flapping_read"]);
}

#[tokio::test]
async fn refused_calls_never_push_the_tries_out() {
    let kb_ = kb();
    tried(&kb_, "a", &["ok"]).await;
    refused(&kb_, "a", "approval_required", ATTEMPTS_PER_ACTION + 10).await;
    let stats = kb_.stats("s").await.unwrap();
    assert_eq!(stats[0].last_verdict.as_deref(), Some("ok"));
    assert!(stats[0].verified);
    assert_eq!(
        kb_.counts("s").await.unwrap().attempts,
        1 + itda_kb::NOT_SENT_PER_ACTION
    );
}

#[tokio::test]
async fn notes_make_room_when_the_site_has_as_many_as_it_keeps() {
    let kb_ = kb();
    let a = Ref {
        kind: "action",
        key: "a",
    };
    for i in 0..itda_kb::FINDINGS_PER_SITE + 3 {
        kb_.finding("s", a, &format!("note {i}"), "setup", 1.0)
            .await
            .unwrap();
    }
    assert_eq!(
        kb_.counts("s").await.unwrap().findings,
        itda_kb::FINDINGS_PER_SITE
    );
    assert_eq!(kb_.search(Some("s"), "note").await.unwrap().len(), 50);
    let newest = format!("note {}", itda_kb::FINDINGS_PER_SITE + 2);
    assert!(kb_.dump().await.unwrap().contains(&newest));
    assert!(!kb_.dump().await.unwrap().contains("\tnote 0\t"));
}

#[tokio::test]
async fn a_key_set_to_null_is_taken_away() {
    let kb_ = kb();
    kb_.node(
        "s",
        "page",
        "/",
        json!({"truncated": true, "fetched": true}),
    )
    .await
    .unwrap();
    kb_.node("s", "page", "/", json!({"truncated": null}))
        .await
        .unwrap();
    let node = &kb_.nodes("s", Some("page"), None).await.unwrap()[0];
    assert_eq!(node.props, json!({"fetched": true}));
    kb_.node("s", "page", "/new", json!({"gone": null}))
        .await
        .unwrap();
    let fresh = &kb_.nodes("s", Some("page"), Some("/new")).await.unwrap()[0];
    assert_eq!(fresh.props, json!({}));
}

#[test]
fn nothing_that_sends_a_request_depends_on_the_knowledge_base() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for crate_ in [
        "net", "browser", "policy", "vault", "spec", "html", "types", "store",
    ] {
        let manifest =
            std::fs::read_to_string(root.join("crates").join(crate_).join("Cargo.toml")).unwrap();
        assert!(
            !manifest.contains("itda-kb"),
            "crates/{crate_} depends on itda-kb"
        );
    }
    // The runtime records tries through the write-only handle and reads
    // nothing: the only names it may use are the recorder's.
    let runtime = std::fs::read_to_string(root.join("crates/runtime/src/lib.rs")).unwrap();
    for forbidden in [
        "Kb",
        ".nodes(",
        ".stats(",
        ".gaps(",
        ".search(",
        ".edges(",
        ".artifacts(",
    ] {
        assert!(
            !runtime.contains(forbidden),
            "runtime uses kb read `{forbidden}`"
        );
    }
    assert!(runtime.contains("Recorder"));
}
