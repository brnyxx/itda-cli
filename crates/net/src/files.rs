//! Files in and files out, streamed with bounded memory.
//!
//! A download goes to a temporary file in its folder and is sealed at EOF:
//! status, media type, size and completeness are checked before the file
//! gets its name, and a failed answer never leaves a partial file behind. An
//! upload is a `multipart/form-data` body streamed from disk in chunks; the
//! bytes sent are hashed as they go and compared with what the person
//! approved, so a file that changed under itda is never sent as that file.
use futures_util::StreamExt;
use itda_types::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How a read saves its answer as a file.
#[derive(Clone, Debug)]
pub struct Download {
    /// The folder the file lands in; created when missing.
    pub dir: PathBuf,
    /// The largest answer accepted, in bytes.
    pub max_bytes: u64,
    /// The media types accepted, lower case (`application/pdf`).
    pub media_types: Vec<String>,
    /// Whether an answer of no bytes counts as a file.
    pub allow_empty: bool,
    /// Only look: an answer whose headers describe the file is read up to
    /// its first bytes, for block classification, and dropped. Nothing is
    /// saved and `dir` is not used.
    pub peek: bool,
}

/// What a download left on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub media_type: String,
    pub sha256: String,
}

/// A `multipart/form-data` body: text fields and at most one file part.
#[derive(Clone)]
pub struct Multipart {
    pub fields: Vec<(String, String)>,
    pub file: Option<FileToSend>,
    /// A seam for tests: called with the bytes sent so far before each
    /// chunk of the file after the first is read, so a test can change
    /// the file between two chunks. `None` in use.
    pub between_chunks: Option<BetweenChunks>,
}

/// What [`Multipart::between_chunks`] calls.
pub type BetweenChunks = Arc<dyn Fn(u64) + Send + Sync>;

impl std::fmt::Debug for Multipart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Multipart")
            .field("fields", &self.fields)
            .field("file", &self.file)
            .finish()
    }
}

/// The file part of an upload, as the person approved it.
#[derive(Clone, Debug)]
pub struct FileToSend {
    /// The form field's name.
    pub field: String,
    pub path: PathBuf,
    /// The name sent as the part's file name.
    pub file_name: String,
    pub media_type: String,
    /// The size and SHA-256 the person approved. The send is aborted when
    /// the bytes read from disk differ.
    pub bytes: u64,
    pub sha256: String,
}

/// Bytes read or written at a time.
const CHUNK: usize = 64 * 1024;
/// How much of an answer is kept in memory for block classification.
pub(crate) const SNIFF: usize = 16 * 1024;
/// A lease is renewed this long after it was taken or last renewed.
pub(crate) const RENEW_EVERY: Duration = Duration::from_secs(30);

/// Why the answer's headers do not describe an acceptable file.
pub(crate) fn refusal(
    download: &Download,
    status: u16,
    content_type: &str,
    content_disposition: &str,
    content_range: &str,
    content_length: Option<u64>,
) -> Option<&'static str> {
    if status == 206 || !content_range.is_empty() {
        return Some("partial_answer");
    }
    let media = media_type(content_type);
    if !download.media_types.contains(&media) {
        return Some("wrong_media_type");
    }
    let page = media == "text/html" || media == "application/xhtml+xml";
    if page && !disposition_is_attachment(content_disposition) {
        return Some("wrong_media_type");
    }
    if content_length.is_some_and(|length| length > download.max_bytes) {
        return Some("too_large");
    }
    if content_length == Some(0) && !download.allow_empty {
        return Some("empty_answer");
    }
    None
}

/// `text/html; charset=utf-8` is `text/html`; no content type is a stream
/// of bytes.
pub(crate) fn media_type(header: &str) -> String {
    let media = header
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if media.is_empty() {
        "application/octet-stream".into()
    } else {
        media
    }
}

fn disposition_is_attachment(header: &str) -> bool {
    header
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("attachment")
}

/// Receive a body into a file under `download.dir`. The file is written
/// under a temporary name and renamed only once the whole answer is in and
/// acceptable; every failure removes it. `leases` are renewed every
/// [`RENEW_EVERY`] while the transfer runs, so a long one keeps its turn.
pub(crate) async fn receive<B: AsRef<[u8]>>(
    download: &Download,
    url: &str,
    content_type: &str,
    content_disposition: &str,
    content_length: Option<u64>,
    mut stream: impl futures_util::Stream<Item = reqwest::Result<B>> + Unpin,
    leases: &[&crate::Lease<'_>],
) -> AppResult<(SavedFile, Vec<u8>)> {
    itda_types::util::private_dir(&download.dir)?;
    let temporary = download
        .dir
        .join(format!(".itda-{}.part", itda_types::util::id()));
    let result = write_to(&temporary, download, content_length, &mut stream, leases).await;
    let (bytes, sha256, sniff) = match result {
        Ok(received) => received,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
    };
    let media = media_type(content_type);
    let path = free_name(&download.dir, &file_name(url, content_disposition, &media));
    if let Err(error) = tokio::fs::rename(&temporary, &path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error.into());
    }
    Ok((
        SavedFile {
            path,
            bytes,
            media_type: media,
            sha256,
        },
        sniff,
    ))
}

async fn write_to<B: AsRef<[u8]>>(
    temporary: &Path,
    download: &Download,
    content_length: Option<u64>,
    stream: &mut (impl futures_util::Stream<Item = reqwest::Result<B>> + Unpin),
    leases: &[&crate::Lease<'_>],
) -> AppResult<(u64, String, Vec<u8>)> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(temporary).await?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut sniff = Vec::new();
    let mut renewed = Instant::now();
    while let Some(chunk) = stream.next().await {
        // A connection that ends before the body does is a short answer.
        let chunk = chunk.map_err(|_| AppError::api("partial_answer", 502))?;
        let chunk = chunk.as_ref();
        bytes += chunk.len() as u64;
        if bytes > download.max_bytes {
            return Err(AppError::api("too_large", 413));
        }
        if sniff.len() < SNIFF {
            sniff.extend_from_slice(&chunk[..chunk.len().min(SNIFF - sniff.len())]);
        }
        hasher.update(chunk);
        file.write_all(chunk).await?;
        if renewed.elapsed() >= RENEW_EVERY {
            for lease in leases {
                lease.renew().await?;
            }
            renewed = Instant::now();
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    if content_length.is_some_and(|length| length != bytes) {
        return Err(AppError::api("partial_answer", 502));
    }
    if bytes == 0 && !download.allow_empty {
        return Err(AppError::api("empty_answer", 502));
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((bytes, sha256, sniff))
}

/// A name for the file: the one the site gave in `Content-Disposition`,
/// else the last part of the address, else `file`, reduced to plain
/// characters so it can never leave its folder. A name without an
/// extension gets one from the media type when one is known.
pub(crate) fn file_name(url: &str, content_disposition: &str, media_type: &str) -> String {
    let given = disposition_file_name(content_disposition)
        .or_else(|| url_file_name(url))
        .unwrap_or_default();
    let mut name = sanitize(&given);
    if name.is_empty() {
        name = "file".into();
    }
    if !name.contains('.')
        && let Some(extension) = extension_of(media_type)
    {
        name.push('.');
        name.push_str(extension);
    }
    name
}

fn disposition_file_name(header: &str) -> Option<String> {
    let mut plain = None;
    for part in header.split(';').skip(1) {
        let (key, value) = part.trim().split_once('=')?;
        let key = key.trim().to_ascii_lowercase();
        if key == "filename*" {
            let encoded = value.trim().rsplit("''").next().unwrap_or_default();
            let decoded = percent_decode(encoded);
            if !decoded.trim().is_empty() {
                return Some(decoded);
            }
        } else if key == "filename" {
            plain = Some(value.trim().trim_matches('"').to_owned());
        }
    }
    plain.filter(|name| !name.trim().is_empty())
}

fn url_file_name(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let segment = parsed
        .path_segments()?
        .rev()
        .find(|segment| !segment.is_empty())?;
    Some(percent_decode(segment))
}

fn percent_decode(text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(value) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Only letters, digits, `-`, `_` and `.`, never a leading dot, at most 100
/// characters: a name that stays in its folder on every platform.
fn sanitize(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut out: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    while out.starts_with(['.', '-', '_']) {
        out.remove(0);
    }
    out
}

/// The media type an upload declares for a file, from its extension.
pub fn media_type_for(path: &Path) -> &'static str {
    itda_types::util::media_type_of(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

fn extension_of(media_type: &str) -> Option<&'static str> {
    Some(match media_type {
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "text/csv" => "csv",
        "text/plain" => "txt",
        "text/html" => "html",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.ms-excel" => "xls",
        "application/msword" => "doc",
        "application/x-hwp" | "application/haansofthwp" => "hwp",
        _ => return None,
    })
}

/// `name.ext`, else `name-2.ext`, `name-3.ext`: a file never replaces one
/// already there.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}{extension}")))
        .find(|path| !path.exists())
        .unwrap_or(candidate)
}

/// A multipart body ready to send: its content type, its length and the
/// stream that produces it. `changed` is set when the file on disk no longer
/// matches what was approved; `complete` when the whole body was produced.
pub(crate) struct MultipartBody {
    pub(crate) content_type: String,
    pub(crate) length: u64,
    pub(crate) body: reqwest::Body,
    pub(crate) changed: Arc<AtomicBool>,
    pub(crate) complete: Arc<AtomicBool>,
}

/// Encode the parts. Text fields go first, the file last, in one stream
/// that reads the file in [`CHUNK`]s and never holds more than that.
pub(crate) async fn multipart_body(parts: &Multipart) -> AppResult<MultipartBody> {
    let boundary = format!("itda-{}{}", itda_types::util::id(), itda_types::util::id());
    let mut head = Vec::new();
    for (name, value) in &parts.fields {
        head.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        head.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
                quoted(name)
            )
            .as_bytes(),
        );
        head.extend_from_slice(value.as_bytes());
        head.extend_from_slice(b"\r\n");
    }
    let tail = format!("\r\n--{boundary}--\r\n").into_bytes();
    let changed = Arc::new(AtomicBool::new(false));
    let complete = Arc::new(AtomicBool::new(false));
    let (length, body) = match &parts.file {
        Some(file) => {
            head.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            head.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                    quoted(&file.field),
                    quoted(&file.file_name),
                    file.media_type
                )
                .as_bytes(),
            );
            let source = Source::open(file).await?;
            let length = head.len() as u64 + file.bytes + tail.len() as u64;
            let stream = source.stream(
                head,
                tail,
                changed.clone(),
                complete.clone(),
                parts.between_chunks.clone(),
            );
            (length, reqwest::Body::wrap_stream(stream))
        }
        None => {
            // No file part: the closing boundary follows the last field
            // without the blank line a part's body ends with.
            let tail = format!("--{boundary}--\r\n").into_bytes();
            let mut whole = head;
            whole.extend_from_slice(&tail);
            complete.store(true, Ordering::SeqCst);
            (whole.len() as u64, reqwest::Body::from(whole))
        }
    };
    Ok(MultipartBody {
        content_type: format!("multipart/form-data; boundary={boundary}"),
        length,
        body,
        changed,
        complete,
    })
}

/// A field or file name inside a `Content-Disposition` header: quotes and
/// line breaks cannot end the header early.
fn quoted(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' => '_',
            c => c,
        })
        .collect()
}

/// The file an upload reads, opened once: the metadata seen then is what
/// every chunk is checked against.
struct Source {
    file: tokio::fs::File,
    bytes: u64,
    modified: Option<std::time::SystemTime>,
    sha256: String,
}

impl Source {
    async fn open(file: &FileToSend) -> AppResult<Self> {
        let handle = tokio::fs::File::open(&file.path)
            .await
            .map_err(|_| AppError::api("data_invalid", 400))?;
        let metadata = handle
            .metadata()
            .await
            .map_err(|_| AppError::api("data_invalid", 400))?;
        if !metadata.is_file() || metadata.len() != file.bytes {
            return Err(AppError::api("data_invalid", 400).with("reason", "file_changed"));
        }
        Ok(Self {
            file: handle,
            bytes: file.bytes,
            modified: metadata.modified().ok(),
            sha256: file.sha256.clone(),
        })
    }

    /// The whole body as a stream: `head`, the file in chunks, `tail`. The
    /// file's size and modification time are checked before every chunk and
    /// its hash at the end; a mismatch ends the stream with an error.
    fn stream(
        self,
        head: Vec<u8>,
        tail: Vec<u8>,
        changed: Arc<AtomicBool>,
        complete: Arc<AtomicBool>,
        between_chunks: Option<BetweenChunks>,
    ) -> impl futures_util::Stream<Item = std::io::Result<Vec<u8>>> {
        enum Stage {
            Head,
            File,
            Tail,
            Done,
        }
        struct State {
            source: Source,
            head: Option<Vec<u8>>,
            tail: Option<Vec<u8>>,
            stage: Stage,
            sent: u64,
            hasher: Sha256,
            changed: Arc<AtomicBool>,
            complete: Arc<AtomicBool>,
            between_chunks: Option<BetweenChunks>,
        }
        let state = State {
            source: self,
            head: Some(head),
            tail: Some(tail),
            stage: Stage::Head,
            sent: 0,
            hasher: Sha256::new(),
            changed,
            complete,
            between_chunks,
        };
        futures_util::stream::try_unfold(state, |mut state| async move {
            let mismatch = |state: &State| {
                state.changed.store(true, Ordering::SeqCst);
                std::io::Error::other("the file changed while it was being sent")
            };
            loop {
                match state.stage {
                    Stage::Head => {
                        state.stage = Stage::File;
                        if let Some(head) = state.head.take() {
                            return Ok(Some((head, state)));
                        }
                    }
                    Stage::File => {
                        if state.sent == state.source.bytes {
                            let digest: String = std::mem::take(&mut state.hasher)
                                .finalize()
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect();
                            if digest != state.source.sha256 {
                                return Err(mismatch(&state));
                            }
                            state.stage = Stage::Tail;
                            continue;
                        }
                        if state.sent > 0
                            && let Some(between) = &state.between_chunks
                        {
                            between(state.sent);
                        }
                        let metadata = state.source.file.metadata().await?;
                        if metadata.len() != state.source.bytes
                            || metadata.modified().ok() != state.source.modified
                        {
                            return Err(mismatch(&state));
                        }
                        let want = (state.source.bytes - state.sent).min(CHUNK as u64) as usize;
                        let mut chunk = vec![0u8; want];
                        let read = state.source.file.read(&mut chunk).await?;
                        if read == 0 {
                            return Err(mismatch(&state));
                        }
                        chunk.truncate(read);
                        state.sent += read as u64;
                        state.hasher.update(&chunk);
                        return Ok(Some((chunk, state)));
                    }
                    Stage::Tail => {
                        state.stage = Stage::Done;
                        state.complete.store(true, Ordering::SeqCst);
                        if let Some(tail) = state.tail.take() {
                            return Ok(Some((tail, state)));
                        }
                    }
                    Stage::Done => return Ok(None),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn names_stay_in_their_folder() {
        assert_eq!(
            file_name(
                "https://a.test/files/7",
                "attachment; filename=\"../../etc/passwd\"",
                "application/pdf"
            ),
            "passwd.pdf"
        );
        assert_eq!(
            file_name(
                "https://a.test/files/7",
                "attachment; filename*=UTF-8''%EB%B3%B4%EA%B3%A0%EC%84%9C.pdf",
                "application/pdf"
            ),
            "보고서.pdf"
        );
        assert_eq!(
            file_name("https://a.test/dl/report%202.csv", "", "text/csv"),
            "report_2.csv"
        );
        assert_eq!(
            file_name("https://a.test/", "", "application/pdf"),
            "file.pdf"
        );
        assert_eq!(
            file_name(
                "https://a.test/x",
                "inline; filename=\".hidden\"",
                "application/x-unknown"
            ),
            "hidden"
        );
        assert_eq!(
            file_name("https://a.test/dl?id=3", "", "application/zip"),
            "dl.zip"
        );
    }

    #[test]
    fn free_names_never_replace_a_file() {
        let dir = std::env::temp_dir().join(format!("itda-files-{}", itda_types::util::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.pdf"), b"x").unwrap();
        std::fs::write(dir.join("a-2.pdf"), b"x").unwrap();
        assert_eq!(free_name(&dir, "a.pdf"), dir.join("a-3.pdf"));
        assert_eq!(free_name(&dir, "b.pdf"), dir.join("b.pdf"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn headers_say_why_an_answer_is_not_the_file() {
        let download = Download {
            dir: PathBuf::new(),
            max_bytes: 100,
            media_types: vec!["application/pdf".into(), "text/html".into()],
            allow_empty: false,
            peek: false,
        };
        let why = |status, kind: &str, disposition: &str, range: &str, length| {
            refusal(&download, status, kind, disposition, range, length)
        };
        assert_eq!(why(200, "application/pdf", "", "", Some(10)), None);
        assert_eq!(why(200, "Application/PDF; x=1", "", "", None), None);
        assert_eq!(
            why(200, "text/csv", "", "", Some(10)),
            Some("wrong_media_type")
        );
        assert_eq!(
            why(200, "text/html", "", "", Some(10)),
            Some("wrong_media_type")
        );
        assert_eq!(
            why(
                200,
                "text/html",
                "attachment; filename=a.html",
                "",
                Some(10)
            ),
            None
        );
        assert_eq!(
            why(206, "application/pdf", "", "bytes 0-9/20", Some(10)),
            Some("partial_answer")
        );
        assert_eq!(
            why(200, "application/pdf", "", "", Some(101)),
            Some("too_large")
        );
        assert_eq!(
            why(200, "application/pdf", "", "", Some(0)),
            Some("empty_answer")
        );
    }
}
