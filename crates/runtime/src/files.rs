//! Files a read saves and files a change sends.
use itda_net::{Download, FileToSend, Multipart, SavedFile};
use itda_spec::{Operation, Spec};
use itda_types::{
    error::{AppError, AppResult, usage},
    util,
};
use serde::Serialize;
use std::path::PathBuf;

use crate::Call;

/// What a file read returns instead of the bytes: where the file is, how
/// big it is, what it is and its SHA-256.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub path: String,
    pub bytes: u64,
    pub media_type: String,
    pub sha256: String,
}

impl Manifest {
    pub fn of(saved: &SavedFile) -> Self {
        Self {
            path: saved.path.to_string_lossy().into_owned(),
            bytes: saved.bytes,
            media_type: saved.media_type.clone(),
            sha256: saved.sha256.clone(),
        }
    }
}

/// The file a change sends, read once before anything is approved: the
/// approval, the audit log and the send all name these bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentFile {
    /// The input the path came in.
    pub input: String,
    pub path: PathBuf,
    /// The file's own name, sent as the part's file name.
    pub name: String,
    pub media_type: String,
    pub bytes: u64,
    pub sha256: String,
}

/// The file the call names in the action's file input, with its size and
/// hash. `None` when the action sends no file. A path that cannot be read
/// as a regular file stops the call before anything is recorded or sent.
pub(crate) async fn sent_file(spec: &Spec, call: &Call) -> AppResult<Option<SentFile>> {
    let Ok(op) = spec.op(&call.action) else {
        return Ok(None);
    };
    let Some(input) = op.file_input() else {
        return Ok(None);
    };
    let Some(given) = call.args.get(input).filter(|p| !p.trim().is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(given);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| {
            usage(
                format!("{input} is not the path of a file"),
                "Give the path of a file on this computer.",
            )
        })?;
    let digest = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || util::file_digest(&path))
            .await
            .map_err(|_| AppError::api("internal", 500))?
    };
    let (bytes, sha256) = digest.map_err(|error| {
        usage(
            format!("{input}: cannot read {given} ({})", error.msg),
            "Give the path of a file on this computer that itda may read.",
        )
    })?;
    Ok(Some(SentFile {
        input: input.clone(),
        media_type: itda_net::files::media_type_for(&path).into(),
        path,
        name,
        bytes,
        sha256,
    }))
}

/// How a file read saves its answer under `dir`, or with `peek` only looks
/// at it.
pub(crate) fn download(op: &Operation, dir: PathBuf, peek: bool) -> Download {
    let response = op.response.as_ref();
    Download {
        dir,
        max_bytes: response.map(|r| r.file_limit()).unwrap_or_default(),
        media_types: response
            .map(|r| {
                r.media_types
                    .iter()
                    .map(|m| m.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default(),
        allow_empty: response.is_some_and(|r| r.allow_empty),
        peek,
    }
}

/// The multipart body to send, when the action has one: its text fields
/// from the inputs and the file read before approval.
pub(crate) fn multipart(
    spec: &Spec,
    op: &Operation,
    args: &std::collections::BTreeMap<String, String>,
    file: Option<&SentFile>,
) -> AppResult<Option<Multipart>> {
    let Some(parts) = spec.multipart_parts(op, args)? else {
        return Ok(None);
    };
    let file = match (parts.file, file) {
        (Some((field, _)), Some(file)) => Some(FileToSend {
            field,
            path: file.path.clone(),
            file_name: file.name.clone(),
            media_type: file.media_type.clone(),
            bytes: file.bytes,
            sha256: file.sha256.clone(),
        }),
        (Some(_), None) => return Err(AppError::api("internal", 500)),
        (None, _) => None,
    };
    Ok(Some(Multipart {
        fields: parts.fields,
        file,
        between_chunks: None,
    }))
}
