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
    pub account: String,
    pub approval: String,
    pub inputs: BTreeMap<String, String>,
    /// `ok` or the error code.
    pub outcome: String,
    pub http_status: u16,
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
    pub prev: String,
    pub hash: String,
}

fn is_zero(value: &u16) -> bool {
    *value == 0
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
            account: event.account,
            approval: event.approval,
            inputs: event.inputs.keys().cloned().collect(),
            inputs_digest: if event.inputs.is_empty() {
                String::new()
            } else {
                util::hash(&serde_json::to_vec(&event.inputs)?)
            },
            outcome: event.outcome,
            http_status: event.http_status,
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
