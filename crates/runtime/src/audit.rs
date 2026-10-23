//! An append-only record of what agents did, one JSON object per line.
//!
//! Each record carries the hash of the one before it, so editing or deleting
//! a line breaks the chain and `verify` names the first broken record.
//! Records hold names, outcomes and a digest of the inputs, never input
//! values or credentials, so the log can be shared when something goes wrong.
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// What happened, before it is numbered and chained.
#[derive(Clone, Debug, Default)]
pub struct Event {
    /// `call`, `change_started`, `approved` or `rejected`.
    pub kind: &'static str,
    pub site: String,
    pub action: String,
    pub effect: String,
    /// Why a declared read was treated as a change, when it was: the
    /// rendered request asked for one (`body names a mutation`).
    pub reclassified: String,
    pub account: String,
    pub approval: String,
    pub inputs: BTreeMap<String, String>,
    /// `ok` or the error code.
    pub outcome: String,
    pub http_status: u16,
    /// For a change whose answer was lost: what the confirming read said,
    /// `landed`, `not_found` or `unknown`, and the read's name.
    pub outcome_check: String,
    pub confirm_read: String,
    /// The SHA-256 of the file a change sent, when it sent one.
    pub file_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub seq: u64,
    pub at: String,
    pub kind: String,
    pub site: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effect: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reclassified: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub account: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub approval: String,
    /// Input names only; the values are covered by `inputs_digest`.
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inputs_digest: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub http_status: u16,
    /// What the confirming read said after a lost answer: `landed`,
    /// `not_found` or `unknown`. Empty when no read was run.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outcome_check: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub confirm_read: String,
    /// The SHA-256 of the file a change sent. Records without one keep
    /// their shape, so a log written before this field verifies as before.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_sha256: String,
    pub prev: String,
    pub hash: String,
}

fn is_zero(value: &u16) -> bool {
    *value == 0
}

/// A digest of the input values, so two calls with the same inputs can be
/// told apart from two with different ones without keeping the values.
fn inputs_digest(inputs: &BTreeMap<String, String>) -> AppResult<String> {
    if inputs.is_empty() {
        return Ok(String::new());
    }
    Ok(util::hash(&serde_json::to_vec(inputs)?))
}

impl Record {
    fn digest(&self) -> AppResult<String> {
        let mut unsigned = self.clone();
        unsigned.hash = String::new();
        Ok(util::hash(&serde_json::to_vec(&unsigned)?))
    }
}

pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    /// The log at `path`; it is created on the first record.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one event. Writers in other processes wait for each other, so
    /// the chain never forks.
    pub fn append(&self, event: Event) -> AppResult<Record> {
        let _lock = util::file_lock(&self.path.with_extension("lock"), false)?;
        let last = self.last()?;
        let mut record = Record {
            seq: last.as_ref().map_or(1, |r| r.seq + 1),
            at: util::now(),
            kind: event.kind.into(),
            site: event.site,
            action: event.action,
            effect: event.effect,
            reclassified: event.reclassified,
            account: event.account,
            approval: event.approval,
            inputs: event.inputs.keys().cloned().collect(),
            inputs_digest: inputs_digest(&event.inputs)?,
            outcome: event.outcome,
            http_status: event.http_status,
            outcome_check: event.outcome_check,
            confirm_read: event.confirm_read,
            file_sha256: event.file_sha256,
            prev: last.map(|r| r.hash).unwrap_or_default(),
            hash: String::new(),
        };
        record.hash = record.digest()?;
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        file.write_all(&line)?;
        file.sync_data()?;
        Ok(record)
    }

    /// Every record, oldest first.
    pub fn records(&self) -> AppResult<Vec<Record>> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        BufReader::new(file)
            .lines()
            .map(|line| Ok(serde_json::from_str(&line?)?))
            .collect()
    }

    /// Changes that were about to be sent and never got an outcome: a
    /// `change_started` record with no later `call` record for the same site,
    /// action and approval. The process stopped after the record was written,
    /// so the site may or may not have applied the change.
    pub fn unfinished_changes(&self) -> AppResult<Vec<Record>> {
        let mut open: Vec<Record> = Vec::new();
        for record in self.records()? {
            match record.kind.as_str() {
                "change_started" => open.push(record),
                "call" => open.retain(|started| {
                    !(started.site == record.site
                        && started.action == record.action
                        && started.approval == record.approval)
                }),
                _ => {}
            }
        }
        Ok(open)
    }

    /// Changes that were sent and whose answer was lost: `call` records
    /// ending in `action_outcome_unknown`, with what the confirming read
    /// said when the change named one.
    pub fn lost_answers(&self) -> AppResult<Vec<Record>> {
        Ok(self
            .records()?
            .into_iter()
            .filter(|record| record.kind == "call" && record.outcome == "action_outcome_unknown")
            .collect())
    }

    /// The last time `action` on `site` as `account` with exactly `inputs`
    /// was sent (a `change_started` record), when its answer was lost and
    /// the confirming read showed it landed. Calls that sent nothing (an
    /// approval asked for, pending or used up) do not count as a later
    /// attempt; a later send with the same inputs does, whatever came of it.
    pub fn landed_before(
        &self,
        site: &str,
        action: &str,
        account: &str,
        inputs: &BTreeMap<String, String>,
    ) -> AppResult<Option<Record>> {
        let digest = inputs_digest(inputs)?;
        let records = self.records()?;
        let Some(sent) = records.iter().rev().find(|record| {
            record.kind == "change_started"
                && record.site == site
                && record.action == action
                && record.account == account
                && record.inputs_digest == digest
        }) else {
            return Ok(None);
        };
        Ok(records
            .iter()
            .find(|record| {
                record.seq > sent.seq
                    && record.kind == "call"
                    && record.site == site
                    && record.action == action
                    && record.approval == sent.approval
            })
            .filter(|record| record.outcome_check == "landed")
            .cloned())
    }

    /// The number of records when the chain is intact, or the first record
    /// that was changed, removed or reordered.
    pub fn verify(&self) -> AppResult<u64> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let (mut prev, mut seq) = (String::new(), 0);
        for line in BufReader::new(file).lines() {
            seq += 1;
            let broken = || {
                let mut error = AppError::api("audit_tampered", 409).with("seq", seq);
                error.msg = format!("The audit log was changed at record {seq}");
                error.remedy =
                    "Keep the file for review; new records continue the chain from the last line."
                        .into();
                error
            };
            let record: Record = serde_json::from_str(&line?).map_err(|_| broken())?;
            if record.seq != seq || record.prev != prev || record.digest()? != record.hash {
                return Err(broken());
            }
            prev = record.hash;
        }
        Ok(seq)
    }

    /// The last record, read from the end of the file.
    fn last(&self) -> AppResult<Option<Record>> {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let length = file.metadata()?.len();
        let start = length.saturating_sub(64 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut tail = String::new();
        file.read_to_string(&mut tail)?;
        match tail.lines().rev().find(|line| !line.trim().is_empty()) {
            Some(line) => Ok(serde_json::from_str(line).ok()),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn log() -> AuditLog {
        let dir = std::env::temp_dir().join(format!("itda-audit-{}", util::id()));
        AuditLog::new(dir.join("audit.jsonl"))
    }

    fn event(title: &str) -> Event {
        Event {
            kind: "call",
            site: "fixture".into(),
            action: "create-item".into(),
            effect: "write".into(),
            inputs: [("title".to_owned(), title.to_owned())].into(),
            outcome: "ok".into(),
            http_status: 201,
            ..Event::default()
        }
    }

    #[test]
    fn records_chain_and_keep_input_values_out() {
        let log = log();
        assert_eq!(log.verify().unwrap(), 0);
        let first = log.append(event("secret plans")).unwrap();
        let second = log.append(event("other")).unwrap();
        assert_eq!((first.seq, second.seq), (1, 2));
        assert_eq!(second.prev, first.hash);
        assert_ne!(first.inputs_digest, second.inputs_digest);
        assert_eq!(log.verify().unwrap(), 2);

        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(!text.contains("secret plans"));
        assert!(text.contains("\"inputs\":[\"title\"]"));
        std::fs::remove_dir_all(log.path().parent().unwrap()).ok();
    }

    #[test]
    fn a_change_without_an_outcome_is_found() {
        let log = log();
        let started = |approval: &str| Event {
            kind: "change_started",
            approval: approval.into(),
            outcome: "sending".into(),
            http_status: 0,
            ..event("x")
        };
        let ended = |approval: &str, outcome: &str| Event {
            approval: approval.into(),
            outcome: outcome.into(),
            ..event("x")
        };
        log.append(started("one")).unwrap();
        log.append(ended("one", "ok")).unwrap();
        log.append(started("two")).unwrap();
        log.append(started("three")).unwrap();
        log.append(ended("three", "action_outcome_unknown"))
            .unwrap();
        let open = log.unfinished_changes().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!((open[0].seq, open[0].approval.as_str()), (3, "two"));
        std::fs::remove_dir_all(log.path().parent().unwrap()).ok();
    }

    #[test]
    fn a_lost_answer_keeps_what_the_confirming_read_said() {
        let log = log();
        let started = |title: &str, approval: &str| Event {
            kind: "change_started",
            approval: approval.into(),
            outcome: "sending".into(),
            http_status: 0,
            ..event(title)
        };
        let lost = |title: &str, approval: &str, check: &str| Event {
            approval: approval.into(),
            outcome: "action_outcome_unknown".into(),
            http_status: 0,
            outcome_check: check.into(),
            confirm_read: "list-items".into(),
            ..event(title)
        };
        let unsent = |title: &str, approval: &str, outcome: &str| Event {
            approval: approval.into(),
            outcome: outcome.into(),
            http_status: 0,
            ..event(title)
        };
        let inputs = |title: &str| BTreeMap::from([("title".to_owned(), title.to_owned())]);
        log.append(started("a", "one")).unwrap();
        log.append(lost("a", "one", "landed")).unwrap();
        // A replay with the used approval and a new request send nothing.
        log.append(unsent("a", "one", "approval_expired")).unwrap();
        log.append(unsent("a", "", "approval_required")).unwrap();
        log.append(started("b", "two")).unwrap();
        log.append(lost("b", "two", "not_found")).unwrap();
        log.append(started("c", "three")).unwrap();
        log.append(lost("c", "three", "landed")).unwrap();
        // Sent again and answered: the person decided about the first one.
        log.append(started("c", "four")).unwrap();
        log.append(Event {
            approval: "four".into(),
            ..event("c")
        })
        .unwrap();
        let lost = log.lost_answers().unwrap();
        assert_eq!(lost.len(), 3);
        assert_eq!(lost[0].outcome_check, "landed");
        assert_eq!(lost[0].confirm_read, "list-items");
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(text.contains("\"outcome_check\":\"landed\""), "{text}");

        let found = log
            .landed_before("fixture", "create-item", "", &inputs("a"))
            .unwrap()
            .unwrap();
        assert_eq!(found.seq, 2);
        assert!(
            log.landed_before("fixture", "create-item", "", &inputs("b"))
                .unwrap()
                .is_none()
        );
        assert!(
            log.landed_before("fixture", "create-item", "", &inputs("c"))
                .unwrap()
                .is_none()
        );
        assert!(
            log.landed_before("fixture", "create-item", "me", &inputs("a"))
                .unwrap()
                .is_none()
        );
        assert_eq!(log.verify().unwrap(), 10);
        std::fs::remove_dir_all(log.path().parent().unwrap()).ok();
    }

    #[test]
    fn records_written_before_the_outcome_check_still_verify() {
        let log = log();
        let first = log.append(event("a")).unwrap();
        // A record from a log written before `outcome_check` existed has no
        // such field; its hash covers what it has.
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(!text.contains("outcome_check"), "{text}");
        assert_eq!(first.outcome_check, "");
        assert_eq!(log.verify().unwrap(), 1);
        std::fs::remove_dir_all(log.path().parent().unwrap()).ok();
    }

    #[test]
    fn edits_and_deletions_break_the_chain() {
        let log = log();
        for title in ["a", "b", "c"] {
            log.append(event(title)).unwrap();
        }
        let original = std::fs::read_to_string(log.path()).unwrap();

        std::fs::write(log.path(), original.replace("\"ok\"", "\"refused\"")).unwrap();
        assert_eq!(log.verify().unwrap_err().extra["seq"], 1);

        let without_second: Vec<&str> = original
            .lines()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, l)| l)
            .collect();
        std::fs::write(log.path(), without_second.join("\n") + "\n").unwrap();
        assert_eq!(log.verify().unwrap_err().extra["seq"], 2);
        std::fs::remove_dir_all(log.path().parent().unwrap()).ok();
    }
}
