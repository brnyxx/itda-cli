//! What the setup, check and refresh steps write into the knowledge base,
//! and what `itda describe`, `itda fsck` and the MCP tools read back from
//! it. Only shapes go in: the actions as the definition declares them, the
//! pages as selectors and field names, the hashes of the files, and one
//! line per read about how it fared. Nothing that was answered.
use crate::check::{self, Outcome};
use itda_kb::{ActionStats, Declared, Gap, Kb, Ref};
use itda_policy::{Effect, operation_effect};
use itda_prep::guide::{Doing, Flow, Guide};
use itda_spec::{Operation, Spec};
use itda_types::{error::AppResult, util};
use serde_json::{Value, json};
use std::path::Path;

/// The line a tool result carries when the knowledge base could not be
/// written (`why` is the error's message). The step it was part of went on
/// without it.
pub fn not_written(why: &str) -> String {
    format!("knowledge base not updated: {why}")
}

/// The longest page key kept, in characters.
const PAGE_KEY: usize = 300;

/// A page as the knowledge base names it: its address without a user name,
/// password, query or fragment, which can carry values (a search, a token),
/// and at most [`PAGE_KEY`] characters long.
pub fn page_key(url: &str) -> String {
    let key = match url::Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.set_fragment(None);
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        }
        Err(_) => url.split(['?', '#']).next().unwrap_or_default().to_owned(),
    };
    key.chars().take(PAGE_KEY).collect()
}

/// The hash and size of a file, or `None` when it is gone.
pub fn hash_file(path: &Path) -> Option<(String, u64)> {
    let bytes = std::fs::read(path).ok()?;
    Some((util::hash(&bytes), bytes.len() as u64))
}

/// Where a file is, as the knowledge base names it: under the data
/// directory, with forward slashes whatever the platform. A file outside it
/// has no name.
fn artifact_name(home: &Path, path: &Path) -> Option<String> {
    Some(
        path.strip_prefix(home)
            .ok()?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn effect_word(op: &Operation) -> &'static str {
    operation_effect(op, &Default::default()).as_str()
}

fn is_read(op: &Operation) -> bool {
    operation_effect(op, &Default::default()) == Effect::Read
}

fn action_ref(name: &str) -> Ref<'_> {
    Ref {
        kind: "action",
        key: name,
    }
}

/// The action nodes of `spec`, as the definition declares them.
pub async fn remember_actions(kb: &Kb, spec: &Spec) -> AppResult<()> {
    for op in &spec.operations {
        kb.node(
            &spec.site.name,
            "action",
            &op.name,
            Declared {
                method: &op.method,
                path: &op.path,
                effect: effect_word(op),
                desc: &op.desc,
                needs: &check::needed_inputs(op),
                confirm: op.confirm.as_ref().map(|c| c.read.as_str()),
                saves_file: op.saves_file(),
                turns_pages: op.pagination.is_some(),
            }
            .props(),
        )
        .await?;
    }
    Ok(())
}

/// Remember the hash of a file the site is made of.
async fn remember_file(kb: &Kb, home: &Path, site: &str, kind: &str, path: &Path) -> AppResult<()> {
    if let (Some((sha256, bytes)), Some(name)) = (hash_file(path), artifact_name(home, path)) {
        kb.artifact(site, kind, &name, &sha256, bytes).await?;
    }
    Ok(())
}

/// Whether a value of the type a shape names (`integer`, `string`) can fill
/// an input declared as `kind` (`int`, `float`, `bool`, `string`).
fn fits(kind: &str, leaf: Option<&Value>) -> bool {
    let Some(leaf) = leaf.and_then(Value::as_str) else {
        return false;
    };
    match kind {
        "int" => leaf == "integer",
        "float" => matches!(leaf, "integer" | "number"),
        "bool" => leaf == "boolean",
        _ => matches!(leaf, "string" | "integer"),
    }
}

/// The declared type of an input of `op`: `string` unless it says.
fn input_kind<'a>(op: &'a Operation, input: &str) -> &'a str {
    op.params
        .get(input)
        .or_else(|| op.query.get(input))
        .or_else(|| {
            op.body
                .iter()
                .flat_map(|body| body.inputs())
                .find(|(name, _)| name.as_str() == input)
                .map(|(_, param)| param)
        })
        .map_or("string", |param| param.kind.as_str())
}

/// The pointer in a list read's answer that could fill `input` (declared
/// as `kind`) of another read, from the shape the list read answered with:
/// `/items/0/id`. The field must have the name and a type that fits; a
/// guess from shapes, never checked against a value.
fn supplies(shape: &Value, input: &str, kind: &str) -> Option<String> {
    let candidates: Vec<(String, &Value)> = match shape {
        Value::Array(items) => items
            .first()
            .map(|first| (String::new(), first))
            .into_iter()
            .collect(),
        Value::Object(map) => map
            .iter()
            .filter_map(|(key, value)| match value {
                Value::Array(items) => items.first().map(|first| (format!("/{key}"), first)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    // `notice_id`, `notice-id` and `noticeId` name a record's `id`; `paid`
    // and `valid` do not.
    let names_an_id = input.eq_ignore_ascii_case("id")
        || input.to_ascii_lowercase().ends_with("_id")
        || input.to_ascii_lowercase().ends_with("-id")
        || (input.ends_with("Id") && input.len() > 2);
    for (base, first) in candidates {
        let Value::Object(record) = first else {
            continue;
        };
        if fits(kind, record.get(input)) {
            return Some(format!("{base}/0/{input}"));
        }
        if names_an_id && fits(kind, record.get("id")) {
            return Some(format!("{base}/0/id"));
        }
    }
    None
}

/// Relate every list read whose answer carries the input of another read:
/// `feeds_input` from the list to the read that needs it.
pub async fn remember_flows(kb: &Kb, spec: &Spec) -> AppResult<()> {
    let site = &spec.site.name;
    let nodes = kb.nodes(site, Some("action"), None).await?;
    let shape_of = |name: &str| -> Option<Value> {
        nodes
            .iter()
            .find(|n| n.key == name)
            .and_then(|n| n.props.get("shape").cloned())
    };
    for detail in spec.operations.iter().filter(|op| is_read(op)) {
        let needs = check::needed_inputs(detail);
        if needs.is_empty() {
            continue;
        }
        for input in &needs {
            let found = spec
                .operations
                .iter()
                .filter(|list| is_read(list) && list.name != detail.name)
                .filter(|list| check::needed_inputs(list).is_empty())
                .find_map(|list| {
                    let kind = input_kind(detail, input);
                    Some((list, supplies(&shape_of(&list.name)?, input, kind)?))
                });
            if let Some((list, pointer)) = found {
                kb.edge(
                    site,
                    action_ref(&list.name),
                    action_ref(&detail.name),
                    "feeds_input",
                    json!({"pointer": pointer, "input": input}),
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// The actions, the flows between them and the hash of the definition.
pub async fn remember_spec(kb: &Kb, home: &Path, spec: &Spec) -> AppResult<()> {
    remember_actions(kb, spec).await?;
    remember_flows(kb, spec).await?;
    if let Some(path) = crate::sites::path(home, &spec.site.name) {
        remember_file(kb, home, &spec.site.name, "spec", &path).await?;
    }
    Ok(())
}

/// The gaps of a site as the rules see them now, with its actions as the
/// definition declares them (a site set up before the knowledge base, or
/// edited by hand, gets them here). The hashes of the files are left as
/// they were, so a file changed under itda still shows.
pub async fn audit(kb: &Kb, home: &Path, spec: &Spec) -> AppResult<Vec<Gap>> {
    remember_actions(kb, spec).await?;
    gaps(kb, home, &spec.site.name).await
}

/// After a check: the definition as it is, the verdict file as written,
/// and the gaps as the rules see them now.
pub async fn remember_check(kb: &Kb, home: &Path, spec: &Spec) -> AppResult<Vec<Gap>> {
    remember_spec(kb, home, spec).await?;
    remember_file(
        kb,
        home,
        &spec.site.name,
        "check",
        &check::check_file(home, &spec.site.name),
    )
    .await?;
    gaps(kb, home, &spec.site.name).await
}

/// Run the rules for `site` against what is on disk now.
pub async fn gaps(kb: &Kb, home: &Path, site: &str) -> AppResult<Vec<Gap>> {
    let recorded = kb.artifacts(site).await?;
    let now = recorded
        .iter()
        .map(|a| {
            let path = home.join(a.path.split('/').collect::<std::path::PathBuf>());
            (a.path.clone(), hash_file(&path).map(|(sha, _)| sha))
        })
        .collect();
    kb.gaps(site, now).await
}

/// How each action of `spec` has fared, in the definition's order: the
/// actions it no longer has are left out.
pub async fn stats(kb: &Kb, spec: &Spec) -> AppResult<Vec<ActionStats>> {
    let all = kb.stats(&spec.site.name).await?;
    Ok(spec
        .operations
        .iter()
        .filter_map(|op| all.iter().find(|s| s.action == op.name).cloned())
        .collect())
}

/// A fetched page: its address and its shape (`itda_html::shape::page`).
pub struct PageShape {
    pub url: String,
    pub shape: Value,
}

/// The pages a site was drafted from, with the actions each one gave.
pub async fn remember_pages(kb: &Kb, spec: &Spec, pages: &[PageShape]) -> AppResult<()> {
    let site = &spec.site.name;
    for page in pages {
        let mut props = page.shape.clone();
        if !props.is_object() {
            props = json!({});
        }
        props["fetched"] = true.into();
        // A shape cut down last time and whole now is no longer truncated.
        if props.get("truncated").is_none() {
            props["truncated"] = Value::Null;
        }
        let key = page_key(&page.url);
        kb.node(site, "page", &key, props).await?;
        let path = url::Url::parse(&page.url)
            .map(|u| u.path().to_owned())
            .unwrap_or_default();
        for op in &spec.operations {
            let from_here = op.path == path
                || (path.len() > 1 && op.evidence.contains(&path))
                || op.evidence.contains(&page.url);
            if from_here {
                kb.edge(
                    site,
                    action_ref(&op.name),
                    Ref {
                        kind: "page",
                        key: &key,
                    },
                    "drafted_from",
                    json!({}),
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// One line per read tried, in itda's words: `kept: ok, HTTP 200, 12
/// records`, `removed: response_mismatch`, `skipped: needs id`. The site's
/// words never appear.
fn setup_finding(outcome: &Outcome, removed: &[String]) -> String {
    match outcome.status.as_str() {
        "ok" => {
            let mut line = format!("kept: ok, {}", outcome.detail);
            if let Some(n) = outcome.records {
                line.push_str(&format!(", {n} records"));
            }
            line
        }
        "skipped" => format!("skipped: {}", outcome.detail),
        code if removed.contains(&outcome.action) => format!("removed: {code}"),
        code => format!("failed: {code}"),
    }
}

/// Everything a finished setup leaves behind: the pages, the actions, the
/// description or software the site was drafted from, one note per read,
/// the definition's hash, and the gaps the rules find. Returns the gaps.
pub async fn remember_setup(
    kb: &Kb,
    home: &Path,
    done: &crate::Done,
    pages: &[PageShape],
    described: Option<&itda_prep::Described>,
) -> AppResult<Vec<Gap>> {
    let site = &done.site;
    remember_actions(kb, &done.spec).await?;
    remember_pages(kb, &done.spec, pages).await?;
    if let Some(described) = described {
        kb.node(
            site,
            "description",
            &page_key(&described.url),
            json!({"reads": described.reads, "changes": described.changes}),
        )
        .await?;
    }
    if let Some(found) = &done.recognised {
        kb.node(
            site,
            "software",
            found.software,
            json!({"version": found.version, "confirmed": found.confirmed}),
        )
        .await?;
    }
    for outcome in &done.outcomes {
        kb.finding(
            site,
            action_ref(&outcome.action),
            &setup_finding(outcome, &done.removed),
            "setup",
            1.0,
        )
        .await?;
    }
    for name in &done.removed {
        kb.node(site, "action", name, json!({"removed": true}))
            .await?;
    }
    remember_spec(kb, home, &done.spec).await?;
    gaps(kb, home, site).await
}

/// After a refresh is written: a changed action's history stays under
/// `name@<hash>` and the new one points back with `replaced_by`; removed
/// and added actions get a note; the definition's hash is renewed.
pub async fn remember_refresh(kb: &Kb, home: &Path, plan: &crate::refresh::Plan) -> AppResult<()> {
    use crate::refresh::Change;
    let site = &plan.site;
    for change in &plan.changes {
        match change {
            Change::Changed { name, fields } => {
                let old = plan.before.operations.iter().find(|op| &op.name == name);
                let hash = old
                    .and_then(|op| serde_json::to_vec(op).ok())
                    .map(|bytes| util::hash(&bytes))
                    .unwrap_or_default();
                let archived = format!("{name}@{}", &hash[..hash.len().min(8)]);
                if let Some(archived) = kb.rename(site, action_ref(name), &archived).await? {
                    kb.node(site, "action", name, json!({})).await?;
                    kb.edge(
                        site,
                        action_ref(&archived),
                        action_ref(name),
                        "replaced_by",
                        json!({"fields": fields}),
                    )
                    .await?;
                }
                kb.finding(
                    site,
                    action_ref(name),
                    &format!("changed: {}", fields.join(", ")),
                    "refresh",
                    1.0,
                )
                .await?;
            }
            Change::Removed { name, why } => {
                kb.node(site, "action", name, json!({"removed": true}))
                    .await?;
                kb.finding(
                    site,
                    action_ref(name),
                    &format!("removed: {why}"),
                    "refresh",
                    1.0,
                )
                .await?;
            }
            Change::Added { name, .. } => {
                kb.finding(site, action_ref(name), "added", "refresh", 1.0)
                    .await?;
            }
            Change::Edited { .. } => {}
        }
    }
    remember_spec(kb, home, &plan.spec).await
}

/// The flows between reads, from the `feeds_input` edges.
pub async fn flows(kb: &Kb, site: &str) -> AppResult<Vec<Flow>> {
    let nodes = kb.nodes(site, Some("action"), None).await?;
    let name = |id: i64| nodes.iter().find(|n| n.id == id).map(|n| n.key.clone());
    Ok(kb
        .edges(site)
        .await?
        .into_iter()
        .filter(|e| e.rel == "feeds_input")
        .filter_map(|e| {
            Some(Flow {
                list: name(e.src)?,
                detail: name(e.dst)?,
                pointer: e.props.get("pointer")?.as_str()?.to_owned(),
                input: e.props.get("input")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// The actions as the guide needs them.
pub fn doings(spec: &Spec) -> Vec<(String, String, bool, Vec<String>)> {
    spec.operations
        .iter()
        .map(|op| {
            (
                op.name.clone(),
                op.desc.clone(),
                is_read(op),
                check::needed_inputs(op),
            )
        })
        .collect()
}

/// The guide and the example prompts for `spec`, in `lang`.
pub fn guide_for(spec: &Spec, flows: &[Flow], lang: &str) -> (Guide, Vec<String>) {
    let rows = doings(spec);
    let doings: Vec<Doing<'_>> = rows
        .iter()
        .map(|(name, desc, read, needs)| Doing {
            name,
            desc,
            read: *read,
            needs,
        })
        .collect();
    let title = if spec.site.title.trim().is_empty() {
        &spec.site.name
    } else {
        &spec.site.title
    };
    (
        itda_prep::guide::guide(&doings, flows, lang),
        itda_prep::guide::prompts(title, &doings, lang),
    )
}

/// The gaps as a result carries them, with the remedy as the next command
/// and the next tool call.
pub fn gaps_json(gaps: &[Gap]) -> Vec<Value> {
    gaps.iter()
        .map(|gap| {
            json!({
                "rule": gap.rule,
                "subject": gap.subject,
                "priority": gap.priority,
                "reason": gap.reason,
                "remedy": {
                    "command": gap.remedy.command,
                    "tool": gap.remedy.tool,
                    "arguments": gap.remedy.arguments,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_reads_shape_says_where_an_input_comes_from() {
        let items = json!({"items": [{"id": "integer", "title": "string"}], "total": "integer"});
        assert_eq!(
            supplies(&items, "id", "int").as_deref(),
            Some("/items/0/id")
        );
        assert_eq!(
            supplies(&items, "notice_id", "string").as_deref(),
            Some("/items/0/id")
        );
        assert_eq!(
            supplies(&items, "noticeId", "int").as_deref(),
            Some("/items/0/id")
        );
        assert_eq!(
            supplies(&items, "title", "string").as_deref(),
            Some("/items/0/title")
        );
        assert_eq!(supplies(&items, "slug", "string"), None);
        assert_eq!(supplies(&items, "paid", "string"), None);
        assert_eq!(supplies(&items, "valid", "string"), None);
        let bare = json!([{"slug": "string"}]);
        assert_eq!(
            supplies(&bare, "slug", "string").as_deref(),
            Some("/0/slug")
        );
        assert_eq!(supplies(&json!("string"), "id", "string"), None);
        // A number in the address is not a node id made of words.
        let nodes = json!({"items": [{"id": "string", "link": "string"}]});
        assert_eq!(supplies(&nodes, "id", "int"), None);
        assert_eq!(supplies(&items, "title", "int"), None);
    }

    #[test]
    fn a_page_is_named_without_its_query() {
        assert_eq!(
            page_key("https://example.com/search?q=private+words&token=abc#top"),
            "https://example.com/search"
        );
        assert_eq!(page_key("not a url?q=1"), "not a url");
        assert_eq!(
            page_key("https://alice:hunter2@example.com/a?b=c"),
            "https://example.com/a"
        );
        let long = format!("https://example.com/{}", "a".repeat(500));
        assert_eq!(page_key(&long).chars().count(), PAGE_KEY);
    }

    #[test]
    fn artifact_names_are_relative_with_forward_slashes() {
        let home = Path::new("/data/itda");
        let name = artifact_name(home, &home.join("specs").join("s").join("spec.yaml"));
        assert_eq!(name.as_deref(), Some("specs/s/spec.yaml"));
        assert_eq!(artifact_name(home, Path::new("/elsewhere/spec.yaml")), None);
    }
}
