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
    /// How many records the answer held, when it was a list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records: Option<usize>,
    /// How long the site asked to wait, when it rate limited the read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_s: Option<u64>,
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
                records: None,
                retry_after_s: None,
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
                Ok(envelope) => Outcome {
                    records: records_in(&envelope.data),
                    ..outcome(
                        "ok",
                        format!("HTTP {}", envelope.meta.http_status),
                        Some(envelope.meta.duration_ms),
                    )
                },
                Err(error) => {
                    let detail = match error.public_code.as_str() {
                        "response_mismatch" => "the answer no longer matches the definition".into(),
                        "auth_required" => format!("sign in: itda login {}", spec.site.name),
                        _ => error.msg.clone(),
                    };
                    Outcome {
                        retry_after_s: error
                            .extra
                            .get("retry_after")
                            .and_then(serde_json::Value::as_u64),
                        ..outcome(&error.public_code, detail, None)
                    }
                }
            });
        }
    }
    out
}

/// The length of the list an answer is, or holds as `items`.
fn records_in(data: &serde_json::Value) -> Option<usize> {
    data.as_array()
        .or_else(|| data.get("items").and_then(serde_json::Value::as_array))
        .map(Vec::len)
}

/// Inputs the action cannot run without: required, or in the path, and with
/// no default.
pub fn needed_inputs(op: &Operation) -> Vec<String> {
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

/// One word per site, worst last: what to do about the site as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    Unreachable,
    Blocked,
    SiteChanged,
    SignedOut,
}

impl Verdict {
    fn of_status(status: &str) -> Option<Self> {
        Some(match status {
            "auth_required" => Self::SignedOut,
            "response_mismatch" | "not_found" => Self::SiteChanged,
            "blocked_by_site" | "site_rate_limited" => Self::Blocked,
            "timeout" | "request_failed" => Self::Unreachable,
            _ => return None,
        })
    }

    /// The worst verdict among these outcomes.
    pub fn of(outcomes: &[Outcome]) -> Self {
        outcomes
            .iter()
            .filter_map(|o| Self::of_status(&o.status))
            .max()
            .unwrap_or(Self::Ok)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unreachable => "unreachable",
            Self::Blocked => "blocked",
            Self::SiteChanged => "site_changed",
            Self::SignedOut => "signed_out",
        }
    }

    /// The error code whose exit code this verdict shares.
    pub fn code(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unreachable => "request_failed",
            Self::Blocked => "blocked_by_site",
            Self::SiteChanged => "response_mismatch",
            Self::SignedOut => "auth_required",
        }
    }

    /// What to do, in plain words.
    pub fn hint(self, site: &str) -> String {
        match self {
            Self::Ok => "every read still matches the site".into(),
            Self::Unreachable => {
                "the site did not answer; check the connection and try later".into()
            }
            Self::Blocked => {
                format!("the site refused; check it in your browser, then itda resume {site}")
            }
            Self::SiteChanged => {
                "the site changed; set it up again with itda setup <url> before an agent uses it"
                    .into()
            }
            Self::SignedOut => format!("sign in again with itda login {site}"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SiteVerdict {
    pub site: String,
    pub verdict: Verdict,
    pub hint: String,
}

/// One verdict per site, in the order the sites were checked.
pub fn verdicts(outcomes: &[Outcome]) -> Vec<SiteVerdict> {
    let mut sites: Vec<&str> = Vec::new();
    for o in outcomes {
        if !sites.contains(&o.site.as_str()) {
            sites.push(&o.site);
        }
    }
    sites
        .into_iter()
        .map(|site| {
            let own: Vec<&Outcome> = outcomes.iter().filter(|o| o.site == site).collect();
            let verdict = own
                .iter()
                .filter_map(|o| Verdict::of_status(&o.status))
                .max()
                .unwrap_or(Verdict::Ok);
            // A failure the verdicts do not cover is not the site's doing,
            // but it must not hide behind "every read still matches".
            let hint = match own.iter().find(|o| o.failed()) {
                Some(other) if verdict == Verdict::Ok => format!(
                    "the site did not change, but {} failed with {}",
                    other.action, other.status
                ),
                _ => verdict.hint(site),
            };
            SiteVerdict {
                site: site.to_owned(),
                verdict,
                hint,
            }
        })
        .collect()
}

/// The code to exit with: the worst verdict, or, when every verdict is ok,
/// the first failure the verdicts do not cover.
pub fn exit_status(outcomes: &[Outcome]) -> &str {
    match Verdict::of(outcomes) {
        Verdict::Ok => outcomes
            .iter()
            .find(|o| o.failed())
            .map_or("ok", |o| o.status.as_str()),
        verdict => verdict.code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(site: &str, status: &str) -> Outcome {
        Outcome {
            site: site.into(),
            action: "a".into(),
            status: status.into(),
            detail: String::new(),
            duration_ms: None,
            records: None,
            retry_after_s: None,
        }
    }

    #[test]
    fn a_signed_out_account_outranks_every_other_verdict() {
        let outcomes = [
            outcome("a", "timeout"),
            outcome("a", "blocked_by_site"),
            outcome("a", "not_found"),
            outcome("a", "auth_required"),
        ];
        assert_eq!(Verdict::of(&outcomes), Verdict::SignedOut);
        assert_eq!(exit_status(&outcomes), "auth_required");
        assert_eq!(Verdict::of(&outcomes[..3]), Verdict::SiteChanged);
        assert_eq!(Verdict::of(&outcomes[..2]), Verdict::Blocked);
        assert_eq!(Verdict::of(&outcomes[..1]), Verdict::Unreachable);
        assert_eq!(Verdict::of(&[outcome("a", "ok")]), Verdict::Ok);
    }

    #[test]
    fn each_site_gets_its_own_verdict_and_other_failures_still_exit() {
        let outcomes = [
            outcome("a", "ok"),
            outcome("b", "response_mismatch"),
            outcome("b", "ok"),
            outcome("c", "internal"),
        ];
        let per_site = verdicts(&outcomes);
        let summary: Vec<(&str, &str)> = per_site
            .iter()
            .map(|v| (v.site.as_str(), v.verdict.as_str()))
            .collect();
        assert_eq!(summary, [("a", "ok"), ("b", "site_changed"), ("c", "ok")]);
        assert!(
            per_site[1].hint.contains("itda setup"),
            "{}",
            per_site[1].hint
        );
        assert_eq!(
            per_site[2].hint,
            "the site did not change, but a failed with internal"
        );
        assert_eq!(exit_status(&outcomes), "response_mismatch");
        assert_eq!(exit_status(&outcomes[3..]), "internal");
    }
}
