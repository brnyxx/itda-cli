//! `itda check`: run every read that needs no input and report whether it
//! still answers the way its definition says, so a site that changed shows up
//! before an agent runs into it.
use itda_policy::{Effect, operation_effect};
use itda_runtime::{Call, Runtime};
use itda_spec::{Operation, Spec};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub site: String,
    pub action: String,
    /// `ok`, `skipped`, or the error code the call ended with.
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

impl Outcome {
    pub fn failed(&self) -> bool {
        !matches!(self.status.as_str(), "ok" | "skipped")
    }
}

/// Check every action of `specs`, one at a time. Changes are never sent.
pub async fn run(runtime: &Runtime, specs: &[Spec], account: Option<&str>) -> Vec<Outcome> {
    let mut out = Vec::new();
    for spec in specs {
        for op in &spec.operations {
            let outcome = |status: &str, detail: String, duration_ms| Outcome {
                site: spec.site.name.clone(),
                action: op.name.clone(),
                status: status.into(),
                detail,
                duration_ms,
            };
            if operation_effect(op, &BTreeMap::new()) != Effect::Read {
                out.push(outcome("skipped", "changes the site".into(), None));
                continue;
            }
            let needed = needed_inputs(op);
            if !needed.is_empty() {
                out.push(outcome(
                    "skipped",
                    format!("needs {}", needed.join(", ")),
                    None,
                ));
                continue;
            }
            let call = Call {
                action: op.name.clone(),
                account: account.map(str::to_owned),
                ..Call::default()
            };
            out.push(match runtime.call(spec, call).await {
                Ok(envelope) => outcome(
                    "ok",
                    format!("HTTP {}", envelope.meta.http_status),
                    Some(envelope.meta.duration_ms),
                ),
                Err(error) => {
                    let detail = match error.public_code.as_str() {
                        "response_mismatch" => "the answer no longer matches the definition".into(),
                        "auth_required" => format!("sign in: itda login {}", spec.site.name),
                        _ => error.msg.clone(),
                    };
                    outcome(&error.public_code, detail, None)
                }
            });
        }
    }
    out
}

/// Inputs the action cannot run without: required, or in the path, and with
/// no default.
fn needed_inputs(op: &Operation) -> Vec<String> {
    let path = op
        .params
        .iter()
        .filter(|(_, p)| p.default.is_none())
        .map(|(name, _)| name.clone());
    let other = op
        .query
        .iter()
        .chain(op.body.iter().flat_map(|body| body.inputs()))
        .filter(|(_, p)| p.required && p.default.is_none())
        .map(|(name, _)| name.clone());
    path.chain(other).collect()
}

/// The code to exit with: a changed site first, then the first other failure.
pub fn worst(outcomes: &[Outcome]) -> Option<&str> {
    outcomes
        .iter()
        .find(|o| o.status == "response_mismatch")
        .or_else(|| outcomes.iter().find(|o| o.failed()))
        .map(|o| o.status.as_str())
}
