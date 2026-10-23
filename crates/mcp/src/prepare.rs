//! `itda_prepare`: one tool for an agent to prepare a site that
//! `itda_setup` could not draft well, by reading its docs and pages as
//! evidence, drafting the definition itself, and letting itda judge, try
//! and install it. The tool list stays the same whatever the op, so prompt
//! caches hold. itda runs no model; the agent is the model.
use crate::{Caller, Server, failure, manage, structured};
use itda_setup::prepare::{Check, Op, Outcome};
use itda_setup::{Context, Stopped};
use itda_types::error::usage;
use rmcp::model::{CallToolResult, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// The longest string a result carries.
const TEXT: usize = 300;
/// The most items of a list a result carries.
const ITEMS: usize = 60;
/// The most keys of an object a result carries.
const KEYS: usize = 60;
/// How deep a result goes.
const DEPTH: usize = 9;

pub(crate) fn tool() -> Tool {
    let schema = json!({"type": "object", "properties": {
        "op": {"type": "string", "enum": ["start", "read", "recon", "search", "session", "propose", "verify", "review", "install"]},
        "session": {"type": "string", "description": "The id start returned; every op but start needs it."},
        "url": {"type": "string", "description": "start: a page of the site. read: a page or document on the site, or on a host it linked as documentation."},
        "name": {"type": "string", "description": "start: the site id to install under; taken from the host when left out."},
        "bundles": {"type": "boolean", "description": "recon: also read the scripts the entry page loads for route strings, each GET tried once. Off unless asked."},
        "q": {"type": "string", "description": "search: words to find in the evidence and in what itda noted about the site."},
        "step": {"type": "string", "enum": ["inspect", "verify", "read"], "description": "session: inspect the site's accounts and credential sources as shapes; verify one signed-in read as the account; read one same-site path with the account's session."},
        "account": {"type": "string", "description": "session: the account id; default when left out."},
        "path": {"type": "string", "description": "session read: a path on the installed site."},
        "understand": {"description": "propose: the workflows the site gives a person, 1 to 12: [{id, title, benefit, steps: [...], success}]."},
        "spec": {"description": "propose: the definition, as a JSON object or YAML text, in the format itda spec validate checks. The site name is the session's."},
        "self_qa": {"type": "array", "description": "propose: up to 12 {lens, question, evidence_ids, answer, action}; kept only when every evidence id exists."},
        "use_recon": {"description": "propose: true, or a list of names, to take operations of the draft recon made whole."},
        "checks": {"type": "array", "items": {"type": "object", "properties": {"action": {"type": "string"}, "inputs": {"type": "object"}}, "required": ["action"]}, "description": "verify: inputs for reads that need them; a value, or $result:<read>:<json pointer> for a scalar an earlier read answered."},
        "workflows": {"type": "array", "description": "review: [{id, steps: [{step, ops: [...]}]}], the operations that cover each step."},
        "replace": {"type": "boolean", "description": "install: replace a site already set up under that name. Off unless the user asked for it."}
    }, "required": ["op"], "additionalProperties": false});
    let schema = match schema {
        Value::Object(map) => Arc::new(map),
        _ => Arc::new(Map::new()),
    };
    Tool::new(
        "itda_prepare",
        "Prepare a site itda_setup could not draft well, as a loop you run: start (a session for the site, with the environment), recon (the entry page, its API description, GraphQL, the routes its docs name and a draft itda made, each piece with an evidence id), read (one more page or document on the site), search (evidence and notes by words), propose (your definition; each operation is kept only when evidence carries it: a documented or answering server, the route in the docs or a page at that path, something to check in an HTML answer), verify (itda tries the kept reads, up to 6 a call and 24 requests a session, and says which answered, with the shape of the answer), review (which kept operations cover each step of each workflow, and which documented routes none implements) and install (the same path as itda_setup: save, try every read once, keep what works). You never see values the site answered with, only keys, types, pointers, selectors, counts and paths; speak of evidence by id. Nothing here approves, signs in or sends a change: changes are written into the definition for the user to approve later. Every result carries feedback with the next call.",
        schema,
    )
    .with_annotations(ToolAnnotations::new().read_only(false).open_world(true))
}

fn text<'a>(arguments: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The op the arguments ask for.
fn op(arguments: &Map<String, Value>) -> Result<Op, itda_types::error::AppError> {
    let needs = |key: &str, what: &str| {
        text(arguments, key)
            .map(str::to_owned)
            .ok_or_else(|| usage(format!("{key} is required"), what.to_owned()))
    };
    Ok(match text(arguments, "op").unwrap_or_default() {
        "start" => Op::Start {
            url: needs("url", "Pass the address of a page on the site.")?,
            name: text(arguments, "name").map(str::to_owned),
        },
        "read" => Op::Read {
            url: needs("url", "Pass the address of a page or document on the site.")?,
        },
        "recon" => Op::Recon {
            bundles: arguments
                .get("bundles")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "search" => Op::Search {
            q: needs("q", "Pass words to look for.")?,
        },
        "session" => Op::Session {
            step: text(arguments, "step").unwrap_or("inspect").to_owned(),
            account: text(arguments, "account").map(str::to_owned),
            path: text(arguments, "path").map(str::to_owned),
        },
        "propose" => Op::Propose {
            understand: arguments.get("understand").cloned(),
            spec: arguments.get("spec").cloned().ok_or_else(|| {
                usage(
                    "spec is required",
                    "Pass the definition: {site: {...}, operations: [...]}.",
                )
            })?,
            self_qa: arguments.get("self_qa").cloned(),
            use_recon: arguments.get("use_recon").cloned(),
        },
        "verify" => Op::Verify {
            checks: arguments
                .get("checks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|check| {
                    let action = check["action"].as_str()?.to_owned();
                    let inputs: BTreeMap<String, String> = check["inputs"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(k, v)| {
                            let value = match v {
                                Value::String(s) => s.clone(),
                                Value::Number(_) | Value::Bool(_) => v.to_string(),
                                _ => return None,
                            };
                            Some((k.clone(), value))
                        })
                        .collect();
                    Some(Check { action, inputs })
                })
                .collect(),
        },
        "review" => Op::Review {
            workflows: arguments.get("workflows").cloned().unwrap_or(Value::Null),
        },
        "install" => Op::Install {
            replace: arguments
                .get("replace")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        other => {
            return Err(usage(
                format!("Unknown op: {other}"),
                "Use start, read, recon, search, session, propose, verify, review or install.",
            ));
        }
    })
}

/// A value cut to the bounds of a result: long strings, long lists, wide
/// objects and deep nesting are shortened, and say so.
fn compact(value: &Value, depth: usize, cut: &mut bool) -> Value {
    match value {
        Value::String(s) if s.chars().count() > TEXT => {
            *cut = true;
            Value::String(format!("{}...", s.chars().take(TEXT).collect::<String>()))
        }
        Value::Array(items) => {
            if depth >= DEPTH {
                *cut = true;
                return json!(format!("array[{}]", items.len()));
            }
            if items.len() > ITEMS {
                *cut = true;
            }
            Value::Array(
                items
                    .iter()
                    .take(ITEMS)
                    .map(|v| compact(v, depth + 1, cut))
                    .collect(),
            )
        }
        Value::Object(map) => {
            if depth >= DEPTH {
                *cut = true;
                return json!(format!("object[{}]", map.len()));
            }
            if map.len() > KEYS {
                *cut = true;
            }
            Value::Object(
                map.iter()
                    .take(KEYS)
                    .map(|(k, v)| (k.clone(), compact(v, depth + 1, cut)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// A result within [`crate::RESULT_LIMIT`]: compacted, then the longest
/// lists halved until it fits, and the feedback kept whatever goes.
pub(crate) fn bounded(value: Value) -> Value {
    let mut cut = false;
    let mut value = compact(&value, 0, &mut cut);
    let fits = |v: &Value| v.to_string().len() <= crate::RESULT_LIMIT * 9 / 10;
    if !fits(&value) {
        let feedback = value.get("feedback").cloned();
        if let Some(map) = value.as_object_mut() {
            map.remove("feedback");
        }
        cut |= crate::shrink_arrays(&mut value, &fits);
        if let (Some(map), Some(feedback)) = (value.as_object_mut(), feedback) {
            map.insert("feedback".into(), feedback);
        }
    }
    if value.to_string().len() > crate::RESULT_LIMIT {
        value = json!({
            "status": value.get("status").cloned().unwrap_or("ok".into()),
            "note": "The result was too large to return; search the evidence by words instead.",
            "feedback": value.get("feedback").cloned().unwrap_or(json!([])),
        });
        cut = true;
    }
    if cut {
        value["truncated"] = true.into();
    }
    value["untrusted"] = true.into();
    value
}

impl Server {
    pub(crate) async fn prepare(
        &self,
        ctx: &Context,
        arguments: &Map<String, Value>,
        caller: Option<&Caller<'_>>,
    ) -> CallToolResult {
        let op = match op(arguments) {
            Ok(op) => op,
            Err(error) => return failure(&error),
        };
        let replace = matches!(op, Op::Install { replace: true });
        let mut answers = manage::Fixed { replace };
        let outcome = self
            .preparer
            .call(ctx, text(arguments, "session"), op, &mut answers)
            .await;
        match outcome {
            Err(error) => failure(&error),
            Ok(Outcome::Reply(value)) => structured(bounded(value)),
            Ok(Outcome::Installed { done, reply }) => {
                if let Err(error) = self.reload(ctx) {
                    return failure(&error);
                }
                self.tools_changed(caller).await;
                let mut result = self.done_json(&done, Vec::new());
                result["session"] = reply["session"].clone();
                result["budgets"] = reply["budgets"].clone();
                result["feedback"] = json!([{
                    "kind": "installed",
                    "subject": done.site,
                    "reason": "the site's actions are tools in this conversation now; reads that did not answer once were left out",
                    "next": {"tool": "itda_sites", "arguments": {"site": done.site}},
                }]);
                structured(bounded(result))
            }
            Ok(Outcome::Stopped { stopped, .. }) => failure(&match stopped {
                Stopped::Exists(site) => manage::coded(
                    "site_exists",
                    409,
                    format!("'{site}' is already set up"),
                    "Call install again with replace set to true if the user wants it replaced.",
                ),
                Stopped::NothingWorks => manage::coded(
                    "nothing_works",
                    422,
                    "No read answered when tried once, so itda kept nothing",
                    "Verify reads until some answer, then install again, or stop and tell the user.",
                ),
                Stopped::NoRecord { .. } => manage::coded(
                    "nothing_works",
                    422,
                    "itda kept nothing",
                    "Stop and tell the user.",
                ),
            }),
        }
    }
}
