//! `itda check`: run every read that needs no input and report whether it
//! still answers the way its definition says, so a site that changed shows up
//! before an agent runs into it.
use itda_kb::Stage;
use itda_policy::{Effect, operation_effect};
use itda_runtime::{Call, Runtime};
use itda_spec::{Operation, Spec};
use itda_types::error::AppResult;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

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
    /// The HTTP status the site answered, when the read ended with one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
}

impl Outcome {
    pub fn failed(&self) -> bool {
        !matches!(self.status.as_str(), "ok" | "skipped")
    }
}

/// The detail of a read `try_once` did not send because the site refused
/// an earlier one.
pub const NOT_TRIED: &str = "not tried: the site refused an earlier read; itda check tries it";

/// Check every action of `specs`, one at a time. Changes are never sent.
pub async fn run(runtime: &Runtime, specs: &[Spec], account: Option<&str>) -> Vec<Outcome> {
    reads(runtime, specs, account, Stage::Check).await
}

/// Try the reads a refresh drafted again before they join the definition,
/// the way `run` does; the knowledge base records them as the refresh's.
pub async fn try_refreshed(runtime: &Runtime, spec: &Spec) -> Vec<Outcome> {
    reads(runtime, std::slice::from_ref(spec), None, Stage::Refresh).await
}

/// Try every read of a site being set up, once, while the person watches. A
/// read the site blocks or challenges is reported without pausing the site,
/// and nothing more goes to the site in this run: the reads after it are
/// `skipped`, so the next `itda check` tries them.
pub async fn try_once(runtime: &Runtime, spec: &Spec) -> Vec<Outcome> {
    reads(runtime, std::slice::from_ref(spec), None, Stage::Setup).await
}

async fn reads(
    runtime: &Runtime,
    specs: &[Spec],
    account: Option<&str>,
    stage: Stage,
) -> Vec<Outcome> {
    let discovery = stage == Stage::Setup;
    let mut out = Vec::new();
    for spec in specs {
        let mut refused = false;
        for op in &spec.operations {
            let outcome = |status: &str, detail: String, duration_ms| Outcome {
                site: spec.site.name.clone(),
                action: op.name.clone(),
                status: status.into(),
                detail,
                duration_ms,
                records: None,
                retry_after_s: None,
                upstream_status: None,
            };
            if operation_effect(op, &BTreeMap::new()) != Effect::Read {
                out.push(outcome("skipped", "changes the site".into(), None));
                continue;
            }
            // A check reads; it does not fill a folder with files. Setup
            // tries a file read once by its headers and first bytes and
            // saves nothing (`Call::discovery`).
            if op.saves_file() && !discovery {
                out.push(outcome("skipped", "saves a file".into(), None));
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
            if refused {
                out.push(outcome("skipped", NOT_TRIED.into(), None));
                continue;
            }
            let call = Call {
                action: op.name.clone(),
                account: account.map(str::to_owned),
                discovery,
                stage,
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
                    let upstream = error
                        .extra
                        .get("upstream_status")
                        .and_then(serde_json::Value::as_u64)
                        .map(|status| status as u16);
                    let detail = match (error.public_code.as_str(), upstream) {
                        ("response_mismatch", _) => {
                            "the answer no longer matches the definition".into()
                        }
                        ("query_failed", _) => "the site answered the query with errors".into(),
                        ("auth_required", _) => format!("sign in: itda login {}", spec.site.name),
                        ("request_failed", Some(status)) => {
                            format!("the site answered HTTP {status}")
                        }
                        // Answered with a block: the site does not give this
                        // page. Refused before sending: the site was paused
                        // already, and nothing more would go out either.
                        ("blocked_by_site", Some(_)) if discovery => {
                            refused = error
                                .extra
                                .get("paused")
                                .and_then(serde_json::Value::as_bool)
                                != Some(false);
                            format!("{}: the site does not give itda this page", op.path)
                        }
                        ("blocked_by_site", None) if discovery => {
                            refused = error
                                .extra
                                .get("paused")
                                .and_then(serde_json::Value::as_bool)
                                != Some(false);
                            error.msg.clone()
                        }
                        _ => error.msg.clone(),
                    };
                    Outcome {
                        retry_after_s: error
                            .extra
                            .get("retry_after")
                            .and_then(serde_json::Value::as_u64),
                        upstream_status: upstream,
                        ..outcome(&error.public_code, detail, None)
                    }
                }
            });
        }
    }
    out
}

/// Whether trying a fresh site left nothing worth keeping: every action
/// failed, or the site has only reads and none of them answered.
pub fn nothing_worked(spec: &Spec, outcomes: &[Outcome]) -> bool {
    let removed = outcomes.iter().filter(|o| o.failed()).count();
    let worked = outcomes.iter().any(|o| o.status == "ok");
    let all_reads = spec
        .operations
        .iter()
        .all(|op| operation_effect(op, &BTreeMap::new()) == Effect::Read);
    spec.operations.len() == removed || (!worked && all_reads)
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    Unreachable,
    Blocked,
    SiteChanged,
    SignedOut,
}

impl Verdict {
    /// What one outcome says about the site. A gone address (404, 405, 410)
    /// and a query the site no longer accepts mean the definition is out of
    /// date, not that the site is down. `api_denied` says nothing about the
    /// site as a whole and is left to the per-action line.
    fn of_outcome(outcome: &Outcome) -> Option<Self> {
        Some(match (outcome.status.as_str(), outcome.upstream_status) {
            ("auth_required", _) => Self::SignedOut,
            ("response_mismatch" | "not_found" | "query_failed" | "data_invalid", _) => {
                Self::SiteChanged
            }
            ("request_failed", Some(404 | 405 | 410)) => Self::SiteChanged,
            ("blocked_by_site" | "site_rate_limited" | "rate_limited", _) => Self::Blocked,
            ("timeout" | "request_failed", _) => Self::Unreachable,
            _ => return None,
        })
    }

    /// The worst verdict among these outcomes.
    pub fn of(outcomes: &[Outcome]) -> Self {
        outcomes
            .iter()
            .filter_map(Self::of_outcome)
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
                format!(
                    "the site refused a read, in the browser too, or asked for a wait; check it in your browser. Only a paused site needs itda resume {site}"
                )
            }
            Self::SiteChanged => {
                format!(
                    "the site changed; run itda refresh {site} to draft it again from the same sources"
                )
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

/// The last verdict a site got, kept next to its definition as
/// `check.json` so `itda_sites` can say when a site was last checked.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LastCheck {
    pub at: String,
    pub verdict: Verdict,
    pub hint: String,
}

/// Keep each site's verdict next to its definition, and in the knowledge
/// base what the check was made of: the actions as the definition declares
/// them, the hashes of the definition and of this verdict, and the gaps
/// the rules find now (the tries themselves were written as each read
/// ran). A site whose definition is gone is skipped. A knowledge base that
/// cannot be written does not fail the check: what went wrong comes back,
/// one message per site, for the caller to show.
pub async fn remember(
    home: &Path,
    kb: &itda_kb::Kb,
    specs: &[Spec],
    verdicts: &[SiteVerdict],
) -> AppResult<Vec<String>> {
    let mut warnings = Vec::new();
    for site in verdicts {
        let dir = crate::sites::specs_dir(home).join(&site.site);
        if !dir.is_dir() {
            continue;
        }
        let last = LastCheck {
            at: itda_types::util::now(),
            verdict: site.verdict,
            hint: site.hint.clone(),
        };
        itda_types::util::atomic_file(&dir.join(CHECK_FILE), &serde_json::to_vec(&last)?)?;
        let Some(spec) = specs.iter().find(|spec| spec.site.name == site.site) else {
            continue;
        };
        if let Err(error) = crate::knowledge::remember_check(kb, home, spec).await {
            warnings.push(error.msg);
        }
    }
    Ok(warnings)
}

/// The file the last verdict is kept in, under the site's directory.
pub fn check_file(home: &Path, site: &str) -> std::path::PathBuf {
    crate::sites::specs_dir(home).join(site).join(CHECK_FILE)
}

/// When the site was last checked and what the verdict was; `None` before
/// its first check.
pub fn last(home: &Path, site: &str) -> Option<LastCheck> {
    let path = crate::sites::specs_dir(home).join(site).join(CHECK_FILE);
    let bytes = itda_types::util::read_bounded(&path, 64 * 1024).ok()?;
    serde_json::from_slice(&bytes).ok()
}

const CHECK_FILE: &str = "check.json";

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
                .filter_map(|o| Verdict::of_outcome(o))
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
            upstream_status: None,
        }
    }

    #[test]
    fn a_gone_address_and_a_failed_query_mean_the_site_changed() {
        let gone = Outcome {
            upstream_status: Some(410),
            ..outcome("a", "request_failed")
        };
        assert_eq!(Verdict::of(&[gone]), Verdict::SiteChanged);
        let down = Outcome {
            upstream_status: Some(502),
            ..outcome("a", "request_failed")
        };
        assert_eq!(Verdict::of(&[down]), Verdict::Unreachable);
        assert_eq!(
            Verdict::of(&[outcome("a", "query_failed")]),
            Verdict::SiteChanged
        );
        // A refused request says nothing about the site as a whole.
        let denied = [outcome("a", "api_denied"), outcome("a", "ok")];
        assert_eq!(Verdict::of(&denied), Verdict::Ok);
        assert_eq!(exit_status(&denied), "api_denied");
        assert!(verdicts(&denied)[0].hint.contains("api_denied"));
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
    fn a_site_that_rate_limits_every_read_is_blocked() {
        let outcomes = [outcome("a", "rate_limited"), outcome("a", "rate_limited")];
        assert_eq!(Verdict::of(&outcomes), Verdict::Blocked);
        let per_site = verdicts(&outcomes);
        assert_eq!(per_site[0].verdict, Verdict::Blocked);
        assert!(
            per_site[0].hint.contains("itda resume a"),
            "{}",
            per_site[0].hint
        );
        assert_eq!(exit_status(&outcomes), "blocked_by_site");
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
            per_site[1].hint.contains("itda refresh b"),
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
