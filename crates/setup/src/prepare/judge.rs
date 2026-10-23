//! `propose`, `verify`, `review`, `session` and `install`: the agent's
//! draft judged against the evidence, its reads tried through the same path
//! every call takes, and the result installed the way `itda setup` does.
//! Changes are judged and written into the draft, never sent.
use super::{
    COVERAGE_ROUNDS, Candidate, Check, Outcome, PER_ROUND, REPAIR_ROUNDS, SELF_QA, SENDS, Session,
    Tried, WORKFLOWS, coded, exhausted, feedback,
    read::{note, turns},
};
use crate::{Answers, Context, Finished, Installing, check, knowledge};
use itda_kb::{Ref, Stage};
use itda_policy::{Effect, operation_effect, rendered_request_effect};
use itda_prep::ground::{self, SeenPath};
use itda_runtime::Call;
use itda_spec::{Operation, Spec};
use itda_types::error::{AppResult, usage};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The largest answer kept in memory for a `$result` chain.
const ANSWER_LIMIT: usize = 1024 * 1024;

fn hash_of(value: &impl serde::Serialize) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    itda_types::util::hash(&bytes)[..16].to_owned()
}

/// What makes two versions of an operation the same read for a check:
/// everything but the pointers a check added and the model it reads past.
fn op_key(op: &Operation) -> String {
    let mut plain = op.clone();
    plain.model.clear();
    if let Some(response) = &mut plain.response {
        response.required_pointers.clear();
    }
    hash_of(&plain)
}

fn endpoint<'a>(key: &'a str) -> Ref<'a> {
    Ref {
        kind: "endpoint",
        key,
    }
}

/// The definition the agent sent, as a value whose site is the session's.
fn draft_value(session: &Session, spec: Value) -> AppResult<Value> {
    let mut value = match spec {
        Value::String(text) => serde_norway::from_str::<Value>(&text).map_err(|error| {
            usage(
                format!("spec does not parse: {error}"),
                "Send the definition as a JSON object, or as YAML text.",
            )
        })?,
        Value::Object(_) => spec,
        _ => {
            return Err(usage(
                "spec must be a definition",
                "Send {site: {...}, operations: [...]} as a JSON object or YAML text.",
            ));
        }
    };
    if !value.is_object() {
        return Err(usage(
            "spec must be a definition",
            "Send {site: {...}, operations: [...]}.",
        ));
    }
    if !value["site"].is_object() {
        value["site"] = json!({});
    }
    value["site"]["name"] = session.site.clone().into();
    if value["site"]["base_url"].as_str().is_none_or(str::is_empty) {
        value["site"]["base_url"] = session.entry.origin().ascii_serialization().into();
    }
    if !value["operations"].is_array() {
        value["operations"] = json!([]);
    }
    Ok(value)
}

/// The operations of itda's own draft the agent takes whole: all of them
/// for `true`, the named ones for a list. An operation of the agent's with
/// the same name wins.
fn add_recon(session: &Session, value: &mut Value, use_recon: Option<&Value>) {
    let Some(recon) = &session.recon else {
        return;
    };
    let wanted = |name: &str| match use_recon {
        Some(Value::Bool(all)) => *all,
        Some(Value::Array(names)) => names.iter().any(|n| n.as_str() == Some(name)),
        _ => false,
    };
    let Some(ops) = value["operations"].as_array_mut() else {
        return;
    };
    let names: Vec<String> = ops
        .iter()
        .filter_map(|op| op["name"].as_str().map(str::to_owned))
        .collect();
    for op in &recon.operations {
        if wanted(&op.name) && !names.contains(&op.name) {
            ops.push(serde_json::to_value(op).unwrap_or_default());
        }
    }
    if let Some(models) = value.get("models").cloned().or_else(|| Some(json!({})))
        && let Value::Object(mut models) = models
    {
        for (name, model) in &recon.models {
            models
                .entry(name.clone())
                .or_insert_with(|| serde_json::to_value(model).unwrap_or_default());
        }
        if !models.is_empty() {
            value["models"] = Value::Object(models);
        }
    }
    if recon.auth.iter().any(|a| a.kind != "none") {
        let auth = value["auth"].as_array().cloned().unwrap_or_default();
        let mut auth = auth;
        for strategy in &recon.auth {
            if !auth
                .iter()
                .any(|a| a["name"].as_str() == Some(&strategy.name))
            {
                auth.push(serde_json::to_value(strategy).unwrap_or_default());
            }
        }
        value["auth"] = Value::Array(auth);
    }
}

/// Workflows as the agent names them: an id, a title, what it is good
/// for, its steps and when it has worked. The ones without an id or a step
/// are refused, with why.
fn workflows(understand: &Value) -> (Vec<Value>, Vec<Value>) {
    let list = understand
        .get("workflows")
        .unwrap_or(understand)
        .as_array()
        .cloned()
        .unwrap_or_default();
    let (mut kept, mut refused) = (Vec::new(), Vec::new());
    for (index, workflow) in list.into_iter().enumerate() {
        let id = workflow["id"].as_str().unwrap_or_default();
        let steps = workflow["steps"].as_array().map_or(0, Vec::len);
        if index >= WORKFLOWS {
            refused
                .push(json!({"index": index, "reason": format!("at most {WORKFLOWS} workflows")}));
        } else if id.is_empty() || steps == 0 {
            refused.push(
                json!({"index": index, "reason": "a workflow has an id and at least one step"}),
            );
        } else {
            kept.push(json!({
                "id": id,
                "title": workflow["title"],
                "benefit": workflow["benefit"],
                "steps": workflow["steps"],
                "success": workflow["success"],
            }));
        }
    }
    (kept, refused)
}

/// The self-questions that point at evidence this session holds; the rest
/// are refused, with why.
fn self_questions(session: &Session, self_qa: &Value) -> (usize, Vec<Value>) {
    let list = self_qa.as_array().cloned().unwrap_or_default();
    let (mut accepted, mut refused) = (0, Vec::new());
    for (index, item) in list.iter().enumerate() {
        let ids: Vec<&str> = item["evidence_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let unknown: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| !session.has_evidence(id))
            .collect();
        if index >= SELF_QA {
            refused.push(json!({"index": index, "reason": format!("at most {SELF_QA} items")}));
        } else if ids.is_empty() {
            refused.push(json!({"index": index, "reason": "no evidence_ids"}));
        } else if !unknown.is_empty() {
            refused.push(json!({"index": index, "reason": format!("unknown evidence {}", unknown.join(", "))}));
        } else {
            accepted += 1;
        }
    }
    (accepted, refused)
}

/// `propose`: the agent's draft judged operation by operation; what is
/// kept becomes the candidate the next steps work on.
pub(crate) async fn propose(
    ctx: &Context,
    session: &mut Session,
    understand: Option<Value>,
    spec: Value,
    self_qa: Option<Value>,
    use_recon: Option<Value>,
) -> AppResult<Value> {
    let mut value = draft_value(session, spec)?;
    add_recon(session, &mut value, use_recon.as_ref());
    let key = format!("propose|{}", hash_of(&json!([value, understand, self_qa])));
    if let Some(previous) = session.done.get(&key) {
        return Ok(json!({
            "status": "ok",
            "already": true,
            "result": previous,
            "feedback": [feedback("already", &session.site, "this proposal was judged already; change it, or verify", json!({"op": "verify", "session": session.id}))],
        }));
    }
    // A proposal after a review that found gaps is a coverage round; after
    // a check that found reads failing, a repair round.
    if session.reviewed {
        if session.coverage_rounds >= COVERAGE_ROUNDS {
            return Err(round_limit(session, "coverage"));
        }
        session.coverage_rounds += 1;
    }
    if session.failing {
        if session.repair_rounds >= REPAIR_ROUNDS {
            return Err(round_limit(session, "repair"));
        }
        session.repair_rounds += 1;
    }
    session.reviewed = false;
    session.failing = false;
    session.proposals += 1;
    let parsed = serde_json::to_vec(&value)
        .map_err(|_| itda_types::error::AppError::api("internal", 500))
        .and_then(|bytes| itda_spec::parse(&bytes));
    let spec = match parsed {
        Ok(spec) => spec,
        Err(error) => {
            return Ok(json!({
                "status": "spec_invalid",
                "message": error.msg,
                "budgets": session.budgets(),
                "feedback": [feedback("spec_invalid", &session.site, "fix the definition's shape and propose again", json!({"op": "propose", "session": session.id}))],
            }));
        }
    };
    let grounded = ground::ground(&spec, &session.seen);
    let site_problems = ground::problems_by_operation(&spec)
        .remove("")
        .unwrap_or_default();
    let kept_names = grounded.kept();
    let mut kept = spec.clone();
    kept.operations
        .retain(|op| kept_names.contains(&op.name.as_str()));
    for op in &mut kept.operations {
        // A declared read the rendered request shows to be a change is one.
        if grounded
            .judgments
            .iter()
            .any(|j| j.op == op.name && j.effect == "change")
            && op.effect == Effect::Read
        {
            op.effect = Effect::Write;
        }
    }
    // What the operation rests on, in the definition's own words, when the
    // agent left it out.
    for op in &mut kept.operations {
        if !op.evidence.trim().is_empty() {
            continue;
        }
        if let Some(j) = grounded.judgments.iter().find(|j| j.op == op.name) {
            let sources: Vec<String> = j
                .evidence_ids
                .iter()
                .filter_map(|id| session.evidence.iter().find(|e| &e.id == id))
                .take(3)
                .map(|e| format!("{} at {}", e.kind.replace('_', " "), e.source))
                .collect();
            op.evidence = format!("itda_prepare: {}", sources.join("; "));
        }
    }
    // A read tried before keeps its result only while it is the same read.
    let hashes: BTreeMap<String, String> = kept
        .operations
        .iter()
        .map(|op| (op.name.clone(), op_key(op)))
        .collect();
    session.tried.retain(|name, tried| {
        hashes.get(name).map(String::as_str) == tried.record["op_hash"].as_str()
    });
    // The pointers a check found stay with the read it found them for.
    for op in &mut kept.operations {
        let added: Vec<String> = session
            .tried
            .get(&op.name)
            .and_then(|t| t.record["pointers_added"].as_array())
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(response) = &mut op.response
            && response.required_pointers.is_empty()
            && !added.is_empty()
        {
            response.required_pointers = added;
        }
    }
    let mut notes = Vec::new();
    for judgment in &grounded.judgments {
        let op = spec.operations.iter().find(|op| op.name == judgment.op);
        let route = op
            .map(|op| {
                format!(
                    "{} {}",
                    if op.method.is_empty() {
                        "GET"
                    } else {
                        &op.method
                    },
                    op.path
                )
            })
            .unwrap_or_default();
        let statement = if judgment.verdict == "kept" {
            format!(
                "prepare: kept {} as a {} ({})",
                judgment.op,
                judgment.effect,
                judgment.evidence_ids.join(", ")
            )
        } else {
            format!("prepare: dropped {}: {}", judgment.op, judgment.reason)
        };
        note(ctx, session, endpoint(&route), &statement).await;
        if judgment.verdict == "dropped" {
            let path = op.map(|op| op.path.clone()).unwrap_or_default();
            notes.push(feedback(
                "dropped",
                &judgment.op,
                &judgment.reason,
                json!({"op": "search", "session": session.id, "q": path}),
            ));
        } else if let Some(read) = &judgment.confirm_with {
            notes.push(feedback(
                "suggest_confirm",
                &judgment.op,
                format!(
                    "give {} confirm: {{read: {read}, match: ...}}, so a lost answer is checked once instead of guessed",
                    judgment.op
                ),
                json!({"op": "propose", "session": session.id}),
            ));
        }
    }
    let (understood, refused_workflows) = understand.as_ref().map(workflows).unwrap_or_default();
    if understand.is_some() {
        session.workflows = understood.clone();
    }
    let (accepted, refused_qa) = self_qa
        .as_ref()
        .map(|qa| self_questions(session, qa))
        .unwrap_or_default();
    if let Some(reason) = &grounded.no_actions {
        notes.push(feedback(
            "no_actions",
            &session.site,
            reason,
            json!({"op": "recon", "session": session.id}),
        ));
    } else if kept_names.is_empty() {
        notes.push(feedback(
            "evidence_free",
            &session.site,
            "no operation rests on evidence; read the docs or the description, then propose what they name",
            json!({"op": "recon", "session": session.id}),
        ));
    } else {
        notes.push(feedback(
            "next",
            &session.site,
            "try the kept reads",
            json!({"op": "verify", "session": session.id}),
        ));
    }
    session.candidate = Some(Candidate {
        spec: kept,
        judgments: grounded.judgments.clone(),
        no_actions: grounded.no_actions.clone(),
    });
    let reply = json!({
        "status": "ok",
        "round": session.proposals,
        "judgments": grounded.judgments,
        "kept": kept_names.len(),
        "dropped": grounded.judgments.len() - kept_names.len(),
        "no_actions": grounded.no_actions,
        "problems": site_problems,
        "understand": {"workflows": understood.len(), "refused": refused_workflows},
        "self_qa": {"accepted": accepted, "refused": refused_qa},
        "budgets": session.budgets(),
        "feedback": notes,
    });
    session.done.insert(key, reply.clone());
    Ok(reply)
}

fn round_limit(session: &Session, kind: &str) -> itda_types::error::AppError {
    let mut error = coded(
        "round_limit",
        429,
        format!("This preparation used its {kind} rounds"),
        "Install what is kept and verified with op install, or stop and tell the user what is missing.",
    );
    error
        .extra
        .insert("session".into(), session.id.clone().into());
    error
}

/// The inputs of one check, with `$result` references filled from reads
/// already tried. `Ok(None)` when a referenced read has not been tried yet.
fn resolve_inputs(
    session: &Session,
    inputs: &BTreeMap<String, String>,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let mut out = BTreeMap::new();
    for (name, value) in inputs {
        let Some((action, pointer)) = ground::result_ref(value) else {
            out.insert(name.clone(), value.clone());
            continue;
        };
        let Some(tried) = session.tried.get(action) else {
            return Ok(None);
        };
        if !tried.answered {
            return Err(format!("{action} did not answer, so {name} has no value"));
        }
        match ground::scalar_at(&tried.answer, pointer) {
            Ok(scalar) => {
                out.insert(name.clone(), scalar);
            }
            Err(why) => {
                return Err(format!(
                    "{action}'s {pointer} is {} for {name}; point at a string, a number or a boolean",
                    match why {
                        "not_scalar" => "a list or an object",
                        "empty" => "empty",
                        _ => "not there",
                    }
                ));
            }
        }
    }
    Ok(Some(out))
}

/// Whether `op` with `args` is a read as itda classifies it and as the
/// request it builds would be sent.
fn rendered_read(spec: &Spec, op: &Operation, args: &BTreeMap<String, String>) -> bool {
    if operation_effect(op, args) != Effect::Read {
        return false;
    }
    spec.build_request(op, args).ok().and_then(|(url, body)| {
        let form = op.body.as_ref().is_some_and(|b| !b.form.is_empty());
        url::Url::parse(&url)
            .ok()
            .map(|url| rendered_request_effect(&url, body.as_deref(), form).effect)
    }) == Some(Effect::Read)
}

/// Records in an answer: its length when it is a list, or the list under
/// `items`.
fn records(data: &Value) -> Option<usize> {
    data.as_array()
        .or_else(|| data.get("items").and_then(Value::as_array))
        .map(Vec::len)
}

/// The draft as checks send it: each read's answer is what its check reads,
/// before any model narrows it.
fn checking_spec(candidate: &Spec) -> Spec {
    let mut spec = candidate.clone();
    for op in &mut spec.operations {
        op.model.clear();
        op.confirm = None;
    }
    spec
}

/// Whether a read's answer is JSON its check reads by pointers.
fn checks_pointers(op: &Operation) -> bool {
    op.response
        .as_ref()
        .is_some_and(|r| r.format == "json" || !r.ssr_json.is_empty())
}

/// `verify`: try the kept reads, at most [`PER_ROUND`] a call and
/// [`SENDS`] requests a session, through the same path every call takes.
/// A read that needs inputs runs only with the inputs a check gives. A
/// change is never sent.
pub(crate) async fn verify(
    ctx: &Context,
    session: &mut Session,
    checks: Vec<Check>,
) -> AppResult<Value> {
    let Some(candidate) = session.candidate.clone() else {
        return Err(coded(
            "nothing_to_verify",
            409,
            "Nothing was proposed yet",
            "Call op propose with a definition first.",
        ));
    };
    let given: BTreeMap<String, BTreeMap<String, String>> =
        checks.into_iter().map(|c| (c.action, c.inputs)).collect();
    let spec = checking_spec(&candidate.spec);
    let mut results = Vec::new();
    let mut skipped = Vec::new();
    let mut notes = Vec::new();
    let mut pending: Vec<&Operation> = Vec::new();
    for op in &candidate.spec.operations {
        if operation_effect(op, &BTreeMap::new()) != Effect::Read {
            skipped.push(json!({"action": op.name, "why": "a change: written into the definition, never sent while preparing"}));
            continue;
        }
        // A read tried before and not named again is not sent again.
        if !given.contains_key(&op.name)
            && let Some(previous) = session.tried.get(&op.name)
            && previous.record["op_hash"].as_str() == Some(op_key(op).as_str())
        {
            let mut record = previous.record.clone();
            record["already"] = true.into();
            results.push(record);
            continue;
        }
        let needed = check::needed_inputs(op);
        let inputs = given.get(&op.name);
        let missing: Vec<&String> = needed
            .iter()
            .filter(|n| inputs.is_none_or(|i| !i.contains_key(*n)))
            .collect();
        if !missing.is_empty() {
            skipped.push(json!({
                "action": op.name,
                "why": format!("needs {}; give it in checks, as a value or as $result:<read>:<pointer>", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")),
            }));
            continue;
        }
        pending.push(op);
    }
    let mut executed = 0;
    let mut progress = true;
    while progress && !pending.is_empty() {
        progress = false;
        let mut later = Vec::new();
        for op in std::mem::take(&mut pending) {
            let inputs = given.get(&op.name).cloned().unwrap_or_default();
            let args = match resolve_inputs(session, &inputs) {
                Ok(Some(args)) => args,
                Ok(None) => {
                    later.push(op);
                    continue;
                }
                Err(why) => {
                    skipped.push(json!({"action": op.name, "why": why}));
                    progress = true;
                    continue;
                }
            };
            progress = true;
            let undeclared: Vec<&str> = args
                .keys()
                .map(String::as_str)
                .filter(|name| op.input(name).is_none())
                .collect();
            if !undeclared.is_empty() {
                skipped.push(json!({"action": op.name, "why": format!("{} is not an input of {}", undeclared.join(", "), op.name)}));
                continue;
            }
            let op_hash = op_key(op);
            let key = format!("verify|{}|{op_hash}|{}", op.name, hash_of(&args));
            if let Some(previous) = session.done.get(&key) {
                let mut record = previous.clone();
                record["already"] = true.into();
                results.push(record);
                notes.push(feedback(
                    "already",
                    &op.name,
                    "tried with the same inputs; not sent again",
                    Value::Null,
                ));
                continue;
            }
            if session.refused {
                skipped.push(json!({"action": op.name, "why": "not tried: the site refused an earlier request in this session"}));
                continue;
            }
            if executed >= PER_ROUND {
                skipped.push(json!({"action": op.name, "why": format!("over {PER_ROUND} reads in one verify; call verify again")}));
                continue;
            }
            if session.sends >= SENDS {
                notes.push(feedback(
                    "budget_exhausted",
                    &op.name,
                    "the check requests of this session are used up",
                    json!({"op": "install", "session": session.id}),
                ));
                skipped.push(
                    json!({"action": op.name, "why": "the session's check requests are used up"}),
                );
                continue;
            }
            // The inputs, then the request as it would be sent, are read
            // once more: a read that asks for a change is one, and is not
            // sent.
            if operation_effect(op, &args) != Effect::Read {
                downgrade(session, &op.name, Effect::Write);
                results.push(json!({"action": op.name, "answered": false, "status": "effect_changed", "reason": "its inputs make it a change", "sent": false, "op_hash": op_hash}));
                continue;
            }
            let mut address = None;
            let rendered = spec.build_request(op, &args).ok().and_then(|(url, body)| {
                let form = op.body.as_ref().is_some_and(|b| !b.form.is_empty());
                url::Url::parse(&url).ok().map(|url| {
                    let effect = rendered_request_effect(&url, body.as_deref(), form);
                    address = Some(url);
                    effect
                })
            });
            match rendered {
                Some(r) if r.effect == Effect::Read => {}
                Some(r) => {
                    let code = if r.effect == Effect::Write {
                        "effect_changed"
                    } else {
                        "effect_unknown"
                    };
                    downgrade(session, &op.name, r.effect);
                    let record = json!({"action": op.name, "answered": false, "status": code, "reason": r.reason, "sent": false, "op_hash": op_hash});
                    results.push(record);
                    continue;
                }
                None => {
                    skipped.push(json!({"action": op.name, "why": "the request could not be built with these inputs"}));
                    continue;
                }
            }
            executed += 1;
            let since = itda_types::util::unix_ms();
            let call = Call {
                action: op.name.clone(),
                args: args.clone(),
                discovery: true,
                stage: Stage::Prepare,
                ..Call::default()
            };
            let outcome = ctx.runtime.call(&spec, call).await;
            let sent = turns(ctx, since).await;
            session.sends += sent;
            let record = match outcome {
                Ok(envelope) => {
                    let data = envelope.data;
                    let answered = if op.saves_file() {
                        true
                    } else {
                        ground::answered(&data)
                    };
                    // Text or a feed answered as declared, but holds no
                    // records a pointer could check.
                    let text = !answered && data.as_str().is_some_and(|t| !t.trim().is_empty());
                    let mut added = Vec::new();
                    if answered
                        && checks_pointers(op)
                        && op
                            .response
                            .as_ref()
                            .is_some_and(|r| r.required_pointers.is_empty())
                    {
                        added = ground::top_pointers(&data);
                        set_pointers(session, &op.name, &added);
                    }
                    let shaped = itda_html::shape::shape(&data);
                    let kept_answer =
                        if serde_json::to_vec(&data).map_or(0, |b| b.len()) <= ANSWER_LIMIT {
                            data.clone()
                        } else {
                            Value::Null
                        };
                    let record = json!({
                        "action": op.name,
                        "answered": answered,
                        "status": if answered { "ok" } else if text { "text" } else { "not_answered" },
                        "http_status": envelope.meta.http_status,
                        "records": records(&data),
                        "shape": shaped,
                        "pointers_added": added,
                        "sent": sent,
                        "op_hash": op_hash,
                    });
                    session.tried.insert(
                        op.name.clone(),
                        Tried {
                            answered,
                            answer: kept_answer,
                            record: record.clone(),
                        },
                    );
                    record
                }
                Err(error) => {
                    let code = error.public_code.clone();
                    let paused = error.extra.get("paused").and_then(Value::as_bool);
                    if (code == "blocked_by_site" && paused != Some(false))
                        || code == "site_rate_limited"
                        || code == "auth_required"
                    {
                        session.refused = true;
                        notes.push(feedback("blocked", &op.name, "the site refused itda; pause and tell the user what it said. Nothing more goes to it in this session", Value::Null));
                    } else if code == "rate_limited" {
                        notes.push(feedback("paced", &op.name, "itda's own pace for this site is used up for now; the site did not refuse", json!({"op": "install", "session": session.id})));
                    }
                    let mut record = json!({
                        "action": op.name,
                        "answered": false,
                        "status": code,
                        "http_status": error.extra.get("upstream_status"),
                        "sent": sent,
                        "op_hash": op_hash,
                    });
                    if code == "response_mismatch"
                        && let Some(diagnosis) = diagnose(session, op, address.as_ref())
                    {
                        record["diagnosis"] = diagnosis;
                    }
                    session.tried.insert(
                        op.name.clone(),
                        Tried {
                            answered: false,
                            answer: Value::Null,
                            record: record.clone(),
                        },
                    );
                    record
                }
            };
            let statement = format!(
                "prepare: tried {}: {}{}",
                op.name,
                record["status"].as_str().unwrap_or_default(),
                record["records"]
                    .as_u64()
                    .map(|n| format!(", {n} records"))
                    .unwrap_or_default()
            );
            let route = format!(
                "{} {}",
                if op.method.is_empty() {
                    "GET"
                } else {
                    &op.method
                },
                op.path
            );
            note(ctx, session, endpoint(&route), &statement).await;
            session.done.insert(key, record.clone());
            results.push(record);
        }
        pending = later;
    }
    for op in pending {
        skipped.push(json!({"action": op.name, "why": "its $result read was not tried; give that read a check too"}));
    }
    let failing = results
        .iter()
        .any(|r| r["answered"] == false && r["status"] != "text" && r["already"].is_null());
    session.failing |= failing;
    notes.push(if failing {
        feedback(
            "repair",
            &session.site,
            "some reads did not answer; drop or fix them and propose again",
            json!({"op": "propose", "session": session.id}),
        )
    } else {
        feedback(
            "next",
            &session.site,
            "say which operations cover each step of each workflow",
            json!({"op": "review", "session": session.id}),
        )
    });
    Ok(json!({
        "status": "ok",
        "results": results,
        "skipped": skipped,
        "budgets": session.budgets(),
        "feedback": notes,
    }))
}

/// Where a page read's extraction misses, measured on the same page as
/// read earlier in the session; counts only.
fn diagnose(session: &Session, op: &Operation, address: Option<&url::Url>) -> Option<Value> {
    let extraction = op.response.as_ref()?.html.as_ref()?;
    let address = address?;
    let page = session.kept_pages.iter().find(|page| {
        url::Url::parse(&page.url).is_ok_and(|url| {
            url.origin() == address.origin()
                && url.path() == address.path()
                && url.query() == address.query()
        })
    })?;
    let mut diagnosis = extraction.diagnose(&page.html);
    diagnosis["measured_on"] = address.path().into();
    Some(diagnosis)
}

/// A read whose rendered request turned out to change something is a
/// change in the draft; one whose effect cannot be told leaves it.
fn downgrade(session: &mut Session, name: &str, effect: Effect) {
    let Some(candidate) = &mut session.candidate else {
        return;
    };
    if effect == Effect::Write {
        if let Some(op) = candidate
            .spec
            .operations
            .iter_mut()
            .find(|op| op.name == name)
        {
            op.effect = Effect::Write;
        }
        if let Some(j) = candidate.judgments.iter_mut().find(|j| j.op == name) {
            j.effect = "change";
        }
    } else {
        candidate.spec.operations.retain(|op| op.name != name);
        if let Some(j) = candidate.judgments.iter_mut().find(|j| j.op == name) {
            j.verdict = "dropped";
            j.reason = "effect_unknown: the request it builds cannot be told a read".into();
        }
    }
}

/// The pointers an answer gave become the read's response check, so it is
/// never installed checking nothing.
fn set_pointers(session: &mut Session, name: &str, pointers: &[String]) {
    if let Some(candidate) = &mut session.candidate
        && let Some(op) = candidate
            .spec
            .operations
            .iter_mut()
            .find(|op| op.name == name)
        && let Some(response) = &mut op.response
    {
        response.required_pointers = pointers.to_vec();
    }
}

/// `review`: which kept operations cover each step of each workflow, and
/// which routes the docs or the description name that no kept operation
/// implements.
pub(crate) fn review(session: &mut Session, workflows: &Value) -> AppResult<Value> {
    let Some(candidate) = session.candidate.clone() else {
        return Err(coded(
            "nothing_to_review",
            409,
            "Nothing was proposed yet",
            "Call op propose with a definition first.",
        ));
    };
    let list = workflows.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return Err(usage(
            "workflows is empty",
            "Pass [{id, steps: [{step, ops: [...]}]}], one entry per workflow understood.",
        ));
    }
    let status_of = |name: &str| -> &'static str {
        let Some(op) = candidate.spec.operations.iter().find(|op| op.name == name) else {
            return "not_kept";
        };
        if operation_effect(op, &BTreeMap::new()) != Effect::Read {
            return "change";
        }
        match session.tried.get(name) {
            Some(t) if t.answered => "answered",
            Some(t) if t.record["status"] == "text" => "text",
            Some(_) => "failed",
            None => "untried",
        }
    };
    let mut rows = Vec::new();
    let mut uncovered = 0;
    for workflow in &list {
        let id = workflow["id"].as_str().unwrap_or_default();
        for (index, step) in workflow["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let ops: Vec<&str> = step["ops"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let statuses: Vec<Value> = ops
                .iter()
                .map(|name| json!({"op": name, "status": status_of(name)}))
                .collect();
            let covered = ops
                .iter()
                .any(|name| matches!(status_of(name), "answered" | "text" | "change"));
            uncovered += usize::from(!covered);
            rows.push(json!({
                "workflow": id,
                "step": step.get("step").cloned().unwrap_or_else(|| index.into()),
                "ops": statuses,
                "covered": covered,
            }));
        }
    }
    let known: Vec<&str> = session
        .workflows
        .iter()
        .filter_map(|w| w["id"].as_str())
        .collect();
    let unknown: Vec<&str> = list
        .iter()
        .filter_map(|w| w["id"].as_str())
        .filter(|id| !known.is_empty() && !known.contains(id))
        .collect();
    // Routes the docs or the description name, with no kept operation.
    let implemented: Vec<(String, String)> = candidate
        .spec
        .operations
        .iter()
        .map(|op| {
            let base = url::Url::parse(candidate.spec.operation_base(op))
                .map(|u| u.path().trim_end_matches('/').to_owned())
                .unwrap_or_default();
            let method = if op.method.is_empty() {
                "GET".to_owned()
            } else {
                op.method.to_ascii_uppercase()
            };
            (
                method,
                format!("{base}/{}", op.path.trim_start_matches('/')),
            )
        })
        .collect();
    let mut unimplemented = Vec::new();
    for route in &session.seen.routes {
        let documented = session.evidence.iter().any(|e| {
            e.id == route.evidence && matches!(e.shown["from"].as_str(), Some("docs" | "openapi"))
        });
        if !documented || route.unsupported_auth {
            continue;
        }
        let done = implemented
            .iter()
            .any(|(m, p)| *m == route.method && ground::route_matches(&route.path, p));
        let listed = unimplemented.iter().any(|v: &Value| {
            v["method"] == route.method.as_str() && v["path"] == route.path.as_str()
        });
        if !done && !listed && unimplemented.len() < 50 {
            unimplemented
                .push(json!({"id": route.evidence, "method": route.method, "path": route.path}));
        }
    }
    session.reviewed = uncovered > 0 || !unimplemented.is_empty();
    let rounds_left = COVERAGE_ROUNDS.saturating_sub(session.coverage_rounds);
    let next = if session.reviewed && rounds_left > 0 {
        feedback(
            "uncovered",
            &session.site,
            format!(
                "{uncovered} steps uncovered and {} documented routes unimplemented; {rounds_left} coverage rounds left",
                unimplemented.len()
            ),
            json!({"op": "propose", "session": session.id}),
        )
    } else {
        feedback(
            "next",
            &session.site,
            "install what is kept; the user is asked only before a site of the same name is replaced",
            json!({"op": "install", "session": session.id}),
        )
    };
    Ok(json!({
        "status": "ok",
        "coverage": rows,
        "uncovered": uncovered,
        "unknown_workflows": unknown,
        "unimplemented_routes": unimplemented,
        "budgets": session.budgets(),
        "feedback": [next],
    }))
}

/// The installed definition of the session's site, for `session`.
fn installed(ctx: &Context, session: &Session) -> AppResult<Spec> {
    crate::sites::load(&ctx.home, &session.site).map_err(|_| {
        coded(
            "not_installed",
            409,
            format!("{} is not installed yet", session.site),
            "Accounts belong to an installed site: install first, have the user sign in with itda_login, then come back.",
        )
    })
}

/// `session`: the site's credentials as shapes (`inspect`), one read as an
/// account (`verify`), or one same-site GET with the account's session
/// (`read`). Only shapes come back.
pub(crate) async fn session(
    ctx: &Context,
    session: &mut Session,
    step: &str,
    account: Option<&str>,
    path: Option<&str>,
) -> AppResult<Value> {
    let account = account.unwrap_or(itda_runtime::DEFAULT_ACCOUNT).to_owned();
    match step {
        "inspect" => {
            let spec = crate::sites::load(&ctx.home, &session.site).ok();
            let accounts: Vec<Value> = ctx
                .vault
                .accounts(Some(&session.site))
                .await?
                .into_iter()
                .map(|a| json!({"id": a.id, "label": a.label, "secrets": a.secrets}))
                .collect();
            let sources: Vec<Value> = spec
                .iter()
                .flat_map(|spec| spec.auth.iter())
                .filter(|a| a.kind != "none")
                .map(|a| json!({"name": a.name, "kind": a.kind, "header": a.header, "value_from": a.value_from}))
                .collect();
            Ok(json!({
                "status": "ok",
                "installed": spec.is_some(),
                "credential_sources": sources,
                "accounts": accounts,
                "budgets": session.budgets(),
                "feedback": [],
            }))
        }
        "verify" | "read" => {
            let spec = installed(ctx, session)?;
            if session.sends >= SENDS {
                return Err(exhausted("check requests"));
            }
            if session.refused {
                return Err(coded(
                    "blocked_by_site",
                    403,
                    "The site refused an earlier request in this session",
                    "Pause and tell the user what it said.",
                ));
            }
            let (spec, action) = if step == "verify" {
                let Some(op) = spec.operations.iter().find(|op| {
                    !op.auth.is_empty()
                        && operation_effect(op, &BTreeMap::new()) == Effect::Read
                        && check::needed_inputs(op).is_empty()
                        && !op.saves_file()
                }) else {
                    return Err(coded(
                        "no_signed_in_read",
                        409,
                        format!("{} has no signed-in read that needs no input", session.site),
                        "Propose one from the docs, install it, then verify it with the account.",
                    ));
                };
                let name = op.name.clone();
                (spec, name)
            } else {
                let Some(path) = path.filter(|p| p.starts_with('/')) else {
                    return Err(usage(
                        "path is required",
                        "Pass a path on the site, such as /account.",
                    ));
                };
                let path = path.split(['?', '#']).next().unwrap_or("/").to_owned();
                let Some(strategy) = spec.auth.iter().find(|a| {
                    a.kind == "browser"
                        || ["cookie:", "storage:", "query:"]
                            .iter()
                            .any(|p| a.value_from.starts_with(p))
                }) else {
                    return Err(coded(
                        "no_session",
                        409,
                        format!("{} signs in with no session", session.site),
                        "Read with op read instead; it needs no account.",
                    ));
                };
                let mut adhoc = spec.clone();
                adhoc.operations = vec![Operation {
                    name: "session-read".into(),
                    method: "GET".into(),
                    path: path.clone(),
                    auth: strategy.name.clone(),
                    effect: Effect::Read,
                    evidence: "a page read with the account's session while preparing".into(),
                    ..Operation::default()
                }];
                (adhoc, "session-read".to_owned())
            };
            let op = spec.op(&action)?.clone();
            // Only a read goes, as it would be sent: anything else would
            // wait for an approval nobody asked for.
            if !rendered_read(&spec, &op, &BTreeMap::new()) {
                return Err(coded(
                    "effect_changed",
                    409,
                    format!("{} {} is not a read", op.method, op.path),
                    "Read a page whose address asks for no change.",
                ));
            }
            let since = itda_types::util::unix_ms();
            let outcome = ctx
                .runtime
                .call(
                    &spec,
                    Call {
                        action: action.clone(),
                        account: Some(account.clone()),
                        discovery: true,
                        stage: Stage::Prepare,
                        ..Call::default()
                    },
                )
                .await;
            let sent = turns(ctx, since).await;
            session.sends += sent;
            let reply = match outcome {
                Ok(envelope) => {
                    let answered = ground::answered(&envelope.data)
                        || envelope.data.as_str().is_some_and(|t| !t.is_empty());
                    if answered && step == "read" {
                        let origin = url::Url::parse(spec.operation_base(&op))
                            .map(|u| u.origin().ascii_serialization())
                            .unwrap_or_default();
                        let id = session.add(
                            "session_page",
                            &op.path,
                            json!({"path": op.path, "account": account}),
                            format!("session {}", op.path),
                        );
                        session.seen.paths.push(SeenPath {
                            origin,
                            path: op.path.trim_end_matches('/').to_owned(),
                            evidence: id,
                            session: true,
                        });
                    }
                    json!({
                        "status": "ok",
                        "action": action,
                        "account": account,
                        "answered": answered,
                        "http_status": envelope.meta.http_status,
                        "shape": itda_html::shape::shape(&envelope.data),
                        "sent": sent,
                    })
                }
                Err(error) => json!({
                    "status": error.public_code,
                    "action": action,
                    "account": account,
                    "answered": false,
                    "sent": sent,
                }),
            };
            let mut reply = reply;
            reply["budgets"] = session.budgets();
            reply["feedback"] = json!([]);
            Ok(reply)
        }
        other => Err(usage(
            format!("Unknown session step: {other}"),
            "Use inspect, verify or read.",
        )),
    }
}

/// `install`: the kept draft saved, every read tried once, what works kept,
/// the way `itda setup` does it. The person is asked only before a site of
/// the same name is replaced (`replace`).
pub(crate) async fn install(
    ctx: &Context,
    session: &mut Session,
    answers: &mut (dyn Answers + Send),
) -> AppResult<Outcome> {
    let Some(candidate) = session.candidate.clone() else {
        return Err(coded(
            "nothing_to_install",
            409,
            "Nothing was proposed yet",
            "Call op propose with a definition first.",
        ));
    };
    if let Some(reason) = &candidate.no_actions {
        return Err(coded(
            "no_actions",
            422,
            format!("Nothing worth installing: {reason}"),
            "Propose reads the evidence names, with something to check in their answers.",
        ));
    }
    if candidate.spec.operations.is_empty() {
        return Err(coded(
            "no_actions",
            422,
            "No operation was kept",
            "Propose operations the evidence names.",
        ));
    }
    let spec = candidate.spec.clone();
    let host = session.entry.host_str().unwrap_or_default().to_owned();
    let body = serde_norway::to_string(&spec)
        .map_err(|_| itda_types::error::AppError::api("internal", 500))?;
    let yaml = format!(
        "# Drafted by an agent with itda_prepare from the evidence of {host}. Review every operation before use:\n# check names, effects and response checks, and remove what agents should not call.\n{body}"
    );
    let reads = spec
        .operations
        .iter()
        .filter(|op| operation_effect(op, &BTreeMap::new()) == Effect::Read)
        .count();
    let problems: Vec<String> = itda_policy::spec_problems(&spec)
        .into_iter()
        .map(|p| format!("{}: {}", p.field, p.detail))
        .collect();
    let draft = itda_prep::Draft {
        site: session.site.clone(),
        yaml,
        operations: spec.operations.len(),
        reads,
        thin_reads: 0,
        notes: Vec::new(),
        problems,
        described: None,
    };
    let shapes = session
        .kept_pages
        .iter()
        .map(|page| knowledge::PageShape {
            url: page.url.clone(),
            shape: itda_html::shape::page(&page.html),
        })
        .collect();
    let installing = Installing {
        spec,
        sources: None,
        recording: None,
        signed: None,
        recognised: None,
        shapes,
    };
    let finished = crate::install(ctx, draft, installing, answers, &mut |_| {}).await?;
    let reply = json!({
        "session": session.id,
        "budgets": session.budgets(),
    });
    Ok(match finished {
        Finished::Done(done) => {
            for op in &done.spec.operations {
                let route = format!(
                    "{} {}",
                    if op.method.is_empty() {
                        "GET"
                    } else {
                        &op.method
                    },
                    op.path
                );
                note(
                    ctx,
                    session,
                    endpoint(&route),
                    &format!("prepare: installed {}", op.name),
                )
                .await;
            }
            Outcome::Installed { done, reply }
        }
        Finished::Stopped(stopped) => Outcome::Stopped { stopped, reply },
    })
}
