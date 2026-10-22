//! `itda refresh <site>`: draft the site again from the sources its
//! definition came from, keep what the person decided, and say what changed.
//!
//! The sources are kept next to the definition as `sources.json` when a
//! draft is installed: the pages, the API description, the GraphQL endpoints
//! asked, a recording kept under the site, and a hash of every operation as
//! it was drafted. An operation whose YAML no longer matches its hash was
//! edited by hand and is kept as it is; the rest follow the new draft.
use crate::{Context, Progress, check, sites};
use itda_prep::Options;
use itda_types::{
    error::{AppError, AppResult, usage},
    util,
};
use serde::{Deserialize, Serialize};
use serde_norway::{Mapping, Value as Yaml};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const SOURCES_FILE: &str = "sources.json";
/// A recording kept under the site for `itda refresh`. It holds the cookies
/// and tokens the recording saw; `itda remove` deletes it with the site.
pub const RECORDING_FILE: &str = "recording.har";
const SOURCES_LIMIT: usize = 1024 * 1024;
const RECORDING_LIMIT: usize = 64 * 1024 * 1024;

/// Where a definition came from, kept next to it as `sources.json`. Names
/// and addresses only, never a value from the site or the keyring.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sources {
    /// The pages the draft was made from, as the person gave them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<String>,
    /// The API description that was read: an address, or a file for
    /// `itda draft openapi`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The GraphQL endpoints asked what they offer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphql: Vec<String>,
    /// A recording kept under the site's directory, by file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
    /// The known-software template the site came from, once there are any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// The choices made for the draft, so it comes out the same way again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lang: String,
    /// Each operation as the sources drafted it, by name: a hash of its
    /// YAML, so one edited by hand can be told apart and kept.
    #[serde(default)]
    pub drafted: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub drafted_at: String,
}

impl Sources {
    /// The sources of a page draft: the pages, what the draft read on the
    /// way, and the options it was made with.
    pub fn of_pages(pages: &[String], draft: &itda_prep::Draft, options: &Options) -> Self {
        Self {
            pages: pages.to_vec(),
            description: draft.described.as_ref().map(|d| d.url.clone()),
            graphql: options.schemas.iter().map(|s| s.url.clone()).collect(),
            base_url: options.base_url.clone(),
            include: options.include.clone(),
            lang: options.lang.clone(),
            ..Self::default()
        }
    }

    /// What the draft was made from, for the words: `pages`, `recording`,
    /// `description` or `template`.
    pub fn kind(&self) -> &'static str {
        if self.template.is_some() {
            "template"
        } else if self.recording.is_some() {
            "recording"
        } else if !self.pages.is_empty() {
            "pages"
        } else if self.description.is_some() {
            "description"
        } else {
            "template"
        }
    }
}

fn sources_path(home: &Path, site: &str) -> PathBuf {
    sites::specs_dir(home).join(site).join(SOURCES_FILE)
}

/// The sources a site was drafted from; `None` for a site installed by hand.
pub fn sources(home: &Path, site: &str) -> Option<Sources> {
    let bytes = util::read_bounded(&sources_path(home, site), SOURCES_LIMIT).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Keep the sources next to the installed definition, with a hash of every
/// operation as it is on disk now. Call it once the definition is final.
pub fn remember(home: &Path, site: &str, mut sources: Sources) -> AppResult<()> {
    let path = sites::path(home, site).ok_or_else(|| AppError::api("internal", 500))?;
    let yaml = String::from_utf8_lossy(&util::read_bounded(&path, RECORDING_LIMIT)?).into_owned();
    sources.drafted = hashes(&yaml)?;
    sources.drafted_at = util::now();
    util::atomic_file(
        &sources_path(home, site),
        &serde_json::to_vec_pretty(&sources)?,
    )
}

/// Keep a recording under the site, for `itda refresh`, with its secrets
/// blanked: cookie, token and session values, private query and body
/// fields, and the answers of sign-in calls. Returns the file name to put
/// in the sources.
pub fn keep_recording(home: &Path, site: &str, har: &[u8]) -> AppResult<String> {
    let path = sites::specs_dir(home).join(site).join(RECORDING_FILE);
    util::atomic_file(&path, &itda_prep::redacted_recording(har)?)?;
    Ok(RECORDING_FILE.to_owned())
}

/// One line of the diff `itda refresh` shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// The sources now draft an action the definition did not have.
    Added { name: String, desc: String },
    /// The action no longer drafts and no longer answers.
    Removed { name: String, why: String },
    /// The sources draft it differently; `fields` names what differs.
    Changed { name: String, fields: Vec<String> },
    /// Edited by hand since it was drafted; kept as it is.
    Edited { name: String },
}

impl Change {
    pub fn name(&self) -> &str {
        match self {
            Self::Added { name, .. }
            | Self::Removed { name, .. }
            | Self::Changed { name, .. }
            | Self::Edited { name } => name,
        }
    }
}

/// What a refresh would do, before anything is written.
pub struct Plan {
    pub site: String,
    pub path: PathBuf,
    /// The definition as installed.
    pub before: itda_spec::Spec,
    /// The merged definition, as it would be written.
    pub yaml: String,
    pub spec: itda_spec::Spec,
    pub changes: Vec<Change>,
    /// The sources to keep, with each operation's new hash.
    pub sources: Sources,
    /// What the check of the installed definition said.
    pub outcomes: Vec<check::Outcome>,
    pub notes: Vec<String>,
    pub problems: Vec<String>,
}

impl Plan {
    /// Whether writing it would change the definition at all.
    pub fn changes_anything(&self) -> bool {
        self.changes
            .iter()
            .any(|change| !matches!(change, Change::Edited { .. }))
    }
}

/// Check the installed definition, draft the site again from its sources
/// and merge the two. Nothing is written.
pub async fn plan(
    ctx: &Context,
    site: &str,
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<Plan> {
    let Some(sources) = sources(&ctx.home, site) else {
        sites::load(&ctx.home, site)?;
        return Err(usage(
            format!("No known sources for '{site}'"),
            "It was installed by hand or by an older itda. Set it up again with `itda setup <url>`.",
        ));
    };
    let before = sites::load(&ctx.home, site)?;
    let path = sites::path(&ctx.home, site).ok_or_else(|| AppError::api("internal", 500))?;
    let installed =
        String::from_utf8_lossy(&util::read_bounded(&path, RECORDING_LIMIT)?).into_owned();
    let outcomes = check::run(&ctx.runtime, std::slice::from_ref(&before), None).await;
    let draft = draft(ctx, site, &before, &sources, report).await?;
    let answers: BTreeMap<String, String> = outcomes
        .iter()
        .map(|o| (o.action.clone(), o.status.clone()))
        .collect();
    let mut merged = merge(&installed, &sources.drafted, &draft.yaml, &answers)?;
    let mut notes = draft.notes;
    let spec = try_added(ctx, &mut merged, &mut notes).await?;
    itda_policy::validate_spec(&spec)?;
    let problems = itda_policy::spec_problems(&spec)
        .into_iter()
        .map(|p| format!("{}: {}", p.field, p.detail))
        .collect();
    Ok(Plan {
        site: site.to_owned(),
        path,
        before,
        yaml: merged.yaml,
        spec,
        changes: merged.changes,
        sources: Sources {
            drafted: merged.drafted,
            ..sources
        },
        outcomes,
        notes,
        problems,
    })
}

/// Try every read the draft adds once, as setup does, and leave out the
/// ones that fail: a read setup already dropped must not come back with
/// every refresh. Returns the merged definition, parsed.
async fn try_added(
    ctx: &Context,
    merged: &mut Merged,
    notes: &mut Vec<String>,
) -> AppResult<itda_spec::Spec> {
    let spec = itda_spec::parse(merged.yaml.as_bytes())?;
    let added: Vec<&str> = merged
        .changes
        .iter()
        .filter_map(|change| match change {
            Change::Added { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    if added.is_empty() {
        return Ok(spec);
    }
    let trial = itda_spec::Spec {
        operations: spec
            .operations
            .iter()
            .filter(|op| added.contains(&op.name.as_str()))
            .cloned()
            .collect(),
        ..spec.clone()
    };
    let tried = check::run(&ctx.runtime, std::slice::from_ref(&trial), None).await;
    let failed: Vec<String> = tried
        .iter()
        .filter(|o| o.failed())
        .map(|o| o.action.clone())
        .collect();
    if failed.is_empty() {
        return Ok(spec);
    }
    for outcome in tried.iter().filter(|o| o.failed()) {
        notes.push(format!(
            "{}: drafted again, but its read failed ({}); left out.",
            outcome.action, outcome.status
        ));
        merged.drafted.remove(&outcome.action);
    }
    merged.changes.retain(|change| {
        !(matches!(change, Change::Added { .. }) && failed.contains(&change.name().to_owned()))
    });
    merged.yaml = crate::without(&merged.yaml, &failed)?;
    itda_spec::parse(merged.yaml.as_bytes())
}

/// Write the merged definition and the sources with the new hashes.
pub fn apply(home: &Path, plan: &Plan) -> AppResult<()> {
    util::atomic_file(&plan.path, plan.yaml.as_bytes())?;
    let sources = Sources {
        drafted_at: util::now(),
        ..plan.sources.clone()
    };
    util::atomic_file(
        &sources_path(home, &plan.site),
        &serde_json::to_vec_pretty(&sources)?,
    )
}

/// Draft the site again from its sources, the way it was drafted the first
/// time.
async fn draft(
    ctx: &Context,
    site: &str,
    before: &itda_spec::Spec,
    sources: &Sources,
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<itda_prep::Draft> {
    let mut options = Options {
        name: Some(site.to_owned()),
        base_url: sources.base_url.clone(),
        include: sources.include.clone(),
        lang: if sources.lang.is_empty() {
            ctx.lang.clone()
        } else {
            sources.lang.clone()
        },
        schemas: Vec::new(),
        session: None,
        frames_to_sign_in: Vec::new(),
    };
    if let Some(file) = &sources.recording {
        let path = sites::specs_dir(&ctx.home).join(site).join(file);
        let har = util::read_bounded(&path, RECORDING_LIMIT).map_err(|_| {
            usage(
                format!("The recording {} is gone", path.display()),
                "Record the site again with `itda record <url> --save <file> --install --force`, or set it up again with `itda setup <url>`.",
            )
        })?;
        let mut endpoints = itda_prep::graphql::endpoints_in_recording(&har);
        endpoints.extend(sources.graphql.iter().cloned());
        endpoints.dedup();
        options.schemas = crate::introspect(ctx, &endpoints, report).await;
        return itda_prep::from_recording(&har, &options, "`itda refresh`");
    }
    // A template site is filled again from the same template at the same
    // API base, with the page read again when it still opens.
    if let Some(name) = &sources.template {
        let definition = crate::software::by_name(name).ok_or_else(|| {
            usage(
                format!(
                    "'{site}' came from a template named {name}, which this itda does not have"
                ),
                "Set it up again with `itda setup <url>`.",
            )
        })?;
        let found = crate::software::Recognised {
            software: definition.name,
            title: if before.site.title.is_empty() {
                definition.title.to_owned()
            } else {
                before.site.title.clone()
            },
            base: sources
                .base_url
                .clone()
                .unwrap_or_else(|| before.site.base_url.clone()),
            version: None,
            confirmed: true,
        };
        report(Progress::Recognised(&found));
        let mut pages = Vec::new();
        for url in &sources.pages {
            let probed = crate::probe(ctx, url, None).await?;
            match (probed.outcome, probed.page) {
                (Ok(()), Some(page)) => {
                    let mut framed = crate::frames::follow(&ctx.net, &page, None).await?;
                    pages.push(page);
                    pages.append(&mut framed.pages);
                }
                (Err((_, Some(refused))), _) => report(Progress::Refused(&refused)),
                _ => {}
            }
        }
        if pages.is_empty() {
            return itda_prep::from_template(&crate::fill(&found), &[], None, &options);
        }
        return crate::draft_pages(
            ctx,
            &pages,
            &sources.graphql,
            &mut options,
            Some(&found),
            report,
        )
        .await;
    }
    if !sources.pages.is_empty() {
        let host = url::Url::parse(&sources.pages[0])
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        report(Progress::Looking { host: &host });
        let mut pages = Vec::new();
        for url in &sources.pages {
            let probed = crate::probe(ctx, url, None).await?;
            match (probed.outcome, probed.page) {
                (Ok(()), Some(page)) => {
                    // The frames a shell showed are followed again, the way
                    // the first draft read them.
                    let mut framed = crate::frames::follow(&ctx.net, &page, None).await?;
                    pages.push(page);
                    pages.append(&mut framed.pages);
                }
                (Err((why, refused)), _) => {
                    if let Some(refused) = &refused {
                        report(Progress::Refused(refused));
                    }
                    return Err(cannot_read(url, why, refused));
                }
                (Ok(()), None) => return Err(AppError::api("internal", 500)),
            }
        }
        return crate::draft_pages(ctx, &pages, &sources.graphql, &mut options, None, report).await;
    }
    if let Some(description) = &sources.description {
        let bytes = if description.starts_with("https://") || description.starts_with("http://") {
            let request = itda_net::Request {
                limit: 4 * 1024 * 1024,
                ..itda_net::Request::read(description)
            };
            let response = ctx.net.fetch(request).await?;
            if !response.successful() {
                return Err(AppError::new(
                    itda_types::error::Code::Runtime,
                    format!("{description} answered HTTP {}", response.status),
                    "Check that the API description is still published there.",
                ));
            }
            response.bytes
        } else {
            util::read_bounded(Path::new(description), 4 * 1024 * 1024)?
        };
        if options.base_url.is_none() {
            options.base_url = Some(before.site.base_url.clone());
        }
        return itda_prep::from_openapi(&bytes, &options);
    }
    Err(usage(
        format!("'{site}' has no sources to draft from again"),
        "Set it up again with `itda setup <url>`.",
    ))
}

/// The error for a page that could not be read as itda, as `itda call`
/// would report it.
fn cannot_read(url: &str, why: crate::Fallback, refused: Option<crate::Refused>) -> AppError {
    if let Some(refused) = refused {
        return refused.error;
    }
    match why {
        crate::Fallback::SignIn => {
            let mut error = AppError::api("auth_required", 401);
            error.msg = format!("{url} needs a sign-in");
            error.remedy = "Record yourself using the site with `itda record <url> --save <file> --install --force`.".into();
            error
        }
        crate::Fallback::Scripted => usage(
            format!("Scripts draw {url}; a plain fetch shows little"),
            "Record yourself using the site with `itda record <url> --save <file> --install --force`.",
        ),
        crate::Fallback::Blocked => AppError::api("blocked_by_site", 403),
    }
}

/// The merged definition and the diff.
#[derive(Debug)]
pub struct Merged {
    pub yaml: String,
    pub changes: Vec<Change>,
    /// Each operation's hash as the sources draft it now.
    pub drafted: BTreeMap<String, String>,
}

/// Merge the installed definition with a new draft. The installed site,
/// `defaults`, `auth` and header comments are kept. Operations are matched
/// by name: one edited by hand (its hash is not in `drafted_before`) is
/// kept as it is; one the draft changed takes the draft's version; one the
/// draft no longer has is removed only when its check (`answers`, by
/// name) failed; the rest of the draft's operations are added.
pub fn merge(
    installed: &str,
    drafted_before: &BTreeMap<String, String>,
    draft: &str,
    answers: &BTreeMap<String, String>,
) -> AppResult<Merged> {
    let (header, mut doc) = split(installed)?;
    let (_, draft_doc) = split(draft)?;
    let installed_ops = operations(&doc);
    let mut draft_ops: Vec<(String, Mapping)> = operations(&draft_doc)
        .into_iter()
        .map(|op| (name_of(&op), op))
        .collect();
    let draft_models = draft_doc
        .get("models")
        .and_then(Yaml::as_mapping)
        .cloned()
        .unwrap_or_default();
    let mut models = doc
        .get("models")
        .and_then(Yaml::as_mapping)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut changes = Vec::new();
    let mut drafted = BTreeMap::new();
    let take_model = |op: &Mapping, models: &mut Mapping| {
        if let Some(model) = op.get("model").and_then(Yaml::as_str)
            && let Some(fields) = draft_models.get(model)
        {
            models.insert(model.into(), fields.clone());
        }
    };
    for op in installed_ops {
        let name = name_of(&op);
        let hash = hash_of(&op);
        let edited = drafted_before.get(&name) != Some(&hash);
        let fresh = draft_ops
            .iter()
            .position(|(n, _)| *n == name)
            .map(|i| draft_ops.remove(i).1);
        match fresh {
            Some(fresh) => {
                let fresh_hash = hash_of(&fresh);
                if edited {
                    changes.push(Change::Edited { name: name.clone() });
                    out.push(op);
                } else if fresh_hash == hash {
                    out.push(op);
                } else {
                    changes.push(Change::Changed {
                        name: name.clone(),
                        fields: differing(&op, &fresh),
                    });
                    take_model(&fresh, &mut models);
                    out.push(fresh);
                }
                drafted.insert(name, fresh_hash);
            }
            None if edited => {
                changes.push(Change::Edited { name });
                out.push(op);
            }
            None => match answers.get(&name).map(String::as_str) {
                Some("ok" | "skipped") | None => {
                    drafted.insert(name, hash);
                    out.push(op);
                }
                Some(status) => changes.push(Change::Removed {
                    name,
                    why: status.to_owned(),
                }),
            },
        }
    }
    for (name, op) in draft_ops {
        changes.push(Change::Added {
            name: name.clone(),
            desc: op
                .get("desc")
                .and_then(Yaml::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
        drafted.insert(name, hash_of(&op));
        take_model(&op, &mut models);
        out.push(op);
    }
    let used: Vec<String> = out
        .iter()
        .filter_map(|op| op.get("model").and_then(Yaml::as_str).map(str::to_owned))
        .collect();
    models.retain(|name, _| name.as_str().is_some_and(|n| used.iter().any(|u| u == n)));
    doc.insert(
        "operations".into(),
        Yaml::Sequence(out.into_iter().map(Yaml::Mapping).collect()),
    );
    if models.is_empty() {
        doc.remove("models");
    } else {
        doc.insert("models".into(), Yaml::Mapping(models));
    }
    let body =
        serde_norway::to_string(&Yaml::Mapping(doc)).map_err(|_| AppError::api("internal", 500))?;
    let yaml = if header.is_empty() {
        body
    } else {
        format!("{header}\n{body}")
    };
    Ok(Merged {
        yaml,
        changes,
        drafted,
    })
}

/// A hash of every operation in a definition, by name, the way the merge
/// compares them: each operation serialized on its own.
pub fn hashes(yaml: &str) -> AppResult<BTreeMap<String, String>> {
    let (_, doc) = split(yaml)?;
    Ok(operations(&doc)
        .iter()
        .map(|op| (name_of(op), hash_of(op)))
        .collect())
}

/// Header comment lines and the document.
fn split(yaml: &str) -> AppResult<(String, Mapping)> {
    let (header, body): (Vec<&str>, Vec<&str>) = yaml.lines().partition(|l| l.starts_with('#'));
    let doc: Yaml = serde_norway::from_str(&body.join("\n")).map_err(|error| {
        itda_types::error::spec(
            format!("itda could not read the definition ({error})"),
            "Fix the YAML, or set the site up again.",
        )
    })?;
    let doc = doc
        .as_mapping()
        .cloned()
        .ok_or_else(|| AppError::api("internal", 500))?;
    Ok((header.join("\n"), doc))
}

fn operations(doc: &Mapping) -> Vec<Mapping> {
    doc.get("operations")
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|op| op.as_mapping().cloned())
        .collect()
}

fn name_of(op: &Mapping) -> String {
    op.get("name")
        .and_then(Yaml::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn hash_of(op: &Mapping) -> String {
    util::hash(
        serde_norway::to_string(&Yaml::Mapping(op.clone()))
            .unwrap_or_default()
            .as_bytes(),
    )
}

/// The top-level fields that differ between two versions of an operation.
fn differing(old: &Mapping, new: &Mapping) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    for key in old
        .iter()
        .chain(new.iter())
        .filter_map(|(key, _)| key.as_str())
    {
        if old.get(key) != new.get(key) && !fields.iter().any(|f| f == key) {
            fields.push(key.to_owned());
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAFTED: &str = "# Drafted by a test.
spec_version: 1
site: {name: board, title: Board, base_url: 'https://board.example'}
defaults: {interval: 2s}
auth:
  - {name: session, kind: browser}
operations:
  - {name: get-board, desc: 'List what Board shows', method: GET, path: /board, effect: read, evidence: 'the page', response: {format: html, html: {items: tr, fields: {title: {selector: 'td:nth-child(1)'}}}}}
  - {name: search, desc: Search, method: GET, path: /search, effect: read, evidence: 'a form', query: {q: {type: string, required: true}}}
  - {name: feed, desc: Feed, method: GET, path: /feed.xml, effect: read, evidence: 'a link', response: {format: xml}}
  - {name: list-things, desc: Things, method: GET, path: /things, effect: read, evidence: 'embedded', model: thing, response: {format: json}}
models:
  thing: {id: {path: id}}
";

    const EDITED: &str = "# Drafted by a test.
spec_version: 1
site: {name: board, title: Board, base_url: 'https://board.example'}
defaults: {interval: 2s}
auth:
  - {name: session, kind: browser}
operations:
  - {name: get-board, desc: 'List what Board shows', method: GET, path: /board, effect: read, evidence: 'the page', response: {format: html, html: {items: tr, fields: {title: {selector: 'td:nth-child(1)'}}}}}
  - {name: search, desc: 'Find notices by title', method: GET, path: /search, effect: read, evidence: 'a form', query: {q: {type: string, default: hi}}, response: {format: html}}
  - {name: feed, desc: Feed, method: GET, path: /feed.xml, effect: read, evidence: 'a link', response: {format: xml}}
  - {name: list-things, desc: Things, method: GET, path: /things, effect: read, evidence: 'embedded', model: thing, response: {format: json}}
  - {name: mine, desc: Mine, method: GET, path: /mine, effect: read, evidence: 'by hand'}
models:
  thing: {id: {path: id}}
";

    const REDRAFTED: &str = "# Drafted by a test, again.
spec_version: 1
site: {name: board-example, base_url: 'https://board.example'}
operations:
  - {name: get-board, desc: 'List what Board shows', method: GET, path: /board, effect: read, evidence: 'the page', response: {format: html, html: {items: tr, fields: {subject: {selector: 'td:nth-child(1)'}}}}}
  - {name: search, desc: Search, method: GET, path: /search, effect: read, evidence: 'a form', query: {q: {type: string, required: true}}}
  - {name: feed, desc: Feed, method: GET, path: /rss.xml, effect: read, evidence: 'a new link', response: {format: xml}}
  - {name: list-notes, desc: Notes, method: GET, path: /notes, effect: read, evidence: 'embedded', model: note, response: {format: json}}
models:
  note: {id: {path: id}, text: {path: body}}
";

    fn answers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn hashes_survive_a_round_trip_through_the_file() {
        let drafted = hashes(DRAFTED).unwrap();
        let doc: Yaml = serde_norway::from_str(
            &DRAFTED
                .lines()
                .filter(|l| !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let written = serde_norway::to_string(&doc).unwrap();
        assert_eq!(hashes(&written).unwrap(), drafted);
        assert_eq!(drafted.len(), 4);
    }

    #[test]
    fn an_unchanged_draft_changes_nothing() {
        let drafted = hashes(DRAFTED).unwrap();
        let merged = merge(DRAFTED, &drafted, DRAFTED, &answers(&[])).unwrap();
        assert!(merged.changes.is_empty(), "{:?}", merged.changes);
        assert_eq!(merged.drafted, drafted);
        assert!(merged.yaml.starts_with("# Drafted by a test.\n"));
        let spec = itda_spec::parse(merged.yaml.as_bytes()).unwrap();
        assert_eq!(spec.operations.len(), 4);
        assert_eq!(spec.site.title, "Board");
    }

    #[test]
    fn hand_edits_survive_and_the_rest_follows_the_draft() {
        let drafted = hashes(DRAFTED).unwrap();
        let merged = merge(
            EDITED,
            &drafted,
            REDRAFTED,
            &answers(&[
                ("get-board", "response_mismatch"),
                ("search", "ok"),
                ("feed", "not_found"),
                ("list-things", "not_found"),
                ("mine", "ok"),
            ]),
        )
        .unwrap();
        let summary: Vec<String> = merged
            .changes
            .iter()
            .map(|c| match c {
                Change::Added { name, .. } => format!("+{name}"),
                Change::Removed { name, why } => format!("-{name}:{why}"),
                Change::Changed { name, fields } => format!("~{name}:{}", fields.join(",")),
                Change::Edited { name } => format!("={name}"),
            })
            .collect();
        assert_eq!(
            summary,
            [
                "~get-board:response",
                "=search",
                "~feed:path,evidence",
                "-list-things:not_found",
                "=mine",
                "+list-notes"
            ]
        );
        let spec = itda_spec::parse(merged.yaml.as_bytes()).unwrap();
        // The site, its defaults and its auth are the installed ones.
        assert_eq!(spec.site.name, "board");
        assert_eq!(spec.site.title, "Board");
        assert_eq!(spec.defaults.interval, "2s");
        assert_eq!(spec.auth[0].name, "session");
        let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["get-board", "search", "feed", "mine", "list-notes"]);
        let search = spec.op("search").unwrap();
        assert_eq!(search.desc, "Find notices by title");
        assert!(search.response.is_some());
        assert_eq!(spec.op("feed").unwrap().path, "/rss.xml");
        let board = spec.op("get-board").unwrap();
        let html = board.response.as_ref().unwrap().html.as_ref().unwrap();
        assert!(html.fields.contains_key("subject"));
        assert!(spec.models.contains_key("note"));
        assert!(!spec.models.contains_key("thing"), "its only read is gone");
        // An edited operation keeps the hash the sources would draft, so it
        // stays edited; one added by hand has none.
        let fresh = hashes(REDRAFTED).unwrap();
        assert_eq!(merged.drafted.get("search"), fresh.get("search"));
        assert!(!merged.drafted.contains_key("mine"));
        assert_eq!(merged.drafted.get("feed"), fresh.get("feed"));
    }

    #[test]
    fn an_operation_that_no_longer_drafts_stays_while_it_answers() {
        let drafted = hashes(DRAFTED).unwrap();
        let merged = merge(
            DRAFTED,
            &drafted,
            REDRAFTED,
            &answers(&[("list-things", "ok"), ("feed", "skipped")]),
        )
        .unwrap();
        let spec = itda_spec::parse(merged.yaml.as_bytes()).unwrap();
        assert!(spec.op("list-things").is_ok());
        assert!(spec.models.contains_key("thing"));
        assert!(merged.drafted.contains_key("list-things"));
        assert!(
            !merged
                .changes
                .iter()
                .any(|c| matches!(c, Change::Removed { .. })),
            "{:?}",
            merged.changes
        );
    }

    #[test]
    fn the_kept_recording_holds_no_values() {
        let home = std::env::temp_dir().join(format!("itda-refresh-{}", util::id()));
        let har = serde_json::json!({"log": {"entries": [{
            "request": {"method": "GET", "url": "https://a.example/api?api_key=K-VALUE&q=hi", "headers": [
                {"name": "Authorization", "value": "Bearer T-VALUE"},
                {"name": "Cookie", "value": "SID=C-VALUE; theme=dark"}
            ]},
            "response": {"status": 200, "headers": [], "content": {"mimeType": "application/json", "text": "{\"items\": []}"}}
        }]}});
        let file = keep_recording(&home, "a", har.to_string().as_bytes()).unwrap();
        assert_eq!(file, RECORDING_FILE);
        let kept = std::fs::read_to_string(home.join("specs/a").join(file)).unwrap();
        for value in ["K-VALUE", "T-VALUE", "C-VALUE"] {
            assert!(!kept.contains(value), "{value} kept:\n{kept}");
        }
        assert!(kept.contains("Bearer <redacted>"), "{kept}");
        assert!(kept.contains("SID=<redacted>; theme=<redacted>"), "{kept}");
        assert!(kept.contains("q=hi"), "{kept}");
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn the_sources_say_what_they_are_made_of() {
        let mut sources = Sources::default();
        assert_eq!(sources.kind(), "template");
        sources.description = Some("https://a.example/openapi.json".into());
        assert_eq!(sources.kind(), "description");
        sources.pages.push("https://a.example/".into());
        assert_eq!(sources.kind(), "pages");
        sources.recording = Some(RECORDING_FILE.into());
        assert_eq!(sources.kind(), "recording");
        let json = serde_json::to_string(&sources).unwrap();
        assert!(!json.contains("template"), "{json}");
        assert!(!json.contains("drafted_at"), "{json}");
    }
}
