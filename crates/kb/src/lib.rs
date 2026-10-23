//! A knowledge base per site, kept in `itda.db` next to the pacing and
//! approval tables. It holds the shape of what itda found and tried: pages
//! and actions as nodes, how they relate as edges, every try with its
//! verdict, notes with a full-text index, and the hashes of the files a
//! site's definition is made of. It never holds a value: no answer, input,
//! cookie, header value, record or form value. Keys, types, pointers,
//! selectors, counts, status codes, times and reasons only.
//!
//! Nothing that sends a request reads it. The runtime gets a [`Recorder`],
//! which can only write a try; the gap rules, the statistics and the shapes
//! are read by `itda fsck`, `itda describe`, `itda ls` and the MCP tools,
//! which give them to a person or an agent as information.
use itda_store::{Migration, Store};
use itda_types::error::{AppError, AppResult};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The kinds a node may have.
pub const KINDS: [&str; 8] = [
    "page",
    "endpoint",
    "action",
    "form",
    "feed",
    "description",
    "graphql_field",
    "software",
];

/// The relations an edge may have.
pub const RELS: [&str; 6] = [
    "links_to",
    "drafted_from",
    "calls",
    "feeds_input",
    "verified_by",
    "replaced_by",
];

/// Tries kept per action; the oldest go first.
pub const ATTEMPTS_PER_ACTION: usize = 200;
/// Nodes kept per site.
pub const NODES_PER_SITE: usize = 2000;
/// Findings kept per site.
pub const FINDINGS_PER_SITE: usize = 2000;
/// How many recent tries the pass rate and the flapping rule look at.
pub const RECENT: usize = 20;
/// The window of the flapping rule: two failures among this many recent
/// tries, with at least one pass, make a read unstable.
pub const FLAP_WINDOW: usize = 7;
/// Verdicts of calls itda refused before anything went to the site: waiting
/// for an approval, an input or a page handle that does not fit, a pacing
/// limit. Such a call carries no HTTP status (a `not_found` the site answered
/// with 404 does, and is a try). They are kept as a record, at most
/// [`NOT_SENT_PER_ACTION`] per action, but are not tries of the site, so the
/// pass rate, the last success and the rules leave them out.
pub const NOT_SENT: [&str; 18] = [
    "approval_required",
    "approval_pending",
    "approval_expired",
    "approval_rejected",
    "writes_disabled",
    "effect_changed",
    "effect_unknown",
    "pagination_expired",
    "pagination_invalid",
    "bad_request",
    "data_invalid",
    "not_found",
    "unsafe_target",
    "local_target",
    "rate_limited",
    "file_changed",
    "bad_url",
    "audit_tampered",
];
/// Calls refused before sending kept per action, beside the tries.
pub const NOT_SENT_PER_ACTION: usize = 20;

/// The SQL condition of a row that is a try of the site, not a call refused
/// before sending. Built from fixed words only.
fn sent() -> String {
    let words: Vec<String> = NOT_SENT.iter().map(|w| format!("'{w}'")).collect();
    format!(
        "NOT (verdict IN ({}) AND http_status IS NULL)",
        words.join(",")
    )
}

static SCHEMA: Migration = Migration {
    component: "kb",
    steps: &[
        "CREATE TABLE kb_node(id INTEGER PRIMARY KEY, site TEXT NOT NULL, kind TEXT NOT NULL, key TEXT NOT NULL, props TEXT NOT NULL DEFAULT '{}', UNIQUE(site,kind,key));
         CREATE TABLE kb_edge(src INTEGER NOT NULL, dst INTEGER NOT NULL, rel TEXT NOT NULL, props TEXT NOT NULL DEFAULT '{}', PRIMARY KEY(src,dst,rel));
         CREATE TABLE kb_attempt(id INTEGER PRIMARY KEY, node INTEGER NOT NULL, stage TEXT NOT NULL, action TEXT NOT NULL, verdict TEXT NOT NULL, http_status INTEGER, duration_ms INTEGER, at TEXT NOT NULL, reason TEXT NOT NULL DEFAULT '');
         CREATE INDEX kb_attempt_node ON kb_attempt(node, id);
         CREATE TABLE kb_gap(id INTEGER PRIMARY KEY, site TEXT NOT NULL, rule TEXT NOT NULL, subject TEXT NOT NULL, state TEXT NOT NULL, priority INTEGER NOT NULL, reason TEXT NOT NULL, UNIQUE(site,rule,subject));
         CREATE TABLE kb_finding(id INTEGER PRIMARY KEY, node INTEGER NOT NULL, statement TEXT NOT NULL, scope TEXT NOT NULL, confidence REAL NOT NULL);
         CREATE VIRTUAL TABLE kb_finding_fts USING fts5(statement, content='kb_finding', content_rowid='id');
         CREATE TRIGGER kb_finding_ai AFTER INSERT ON kb_finding BEGIN INSERT INTO kb_finding_fts(rowid, statement) VALUES (new.id, new.statement); END;
         CREATE TRIGGER kb_finding_ad AFTER DELETE ON kb_finding BEGIN INSERT INTO kb_finding_fts(kb_finding_fts, rowid, statement) VALUES ('delete', old.id, old.statement); END;
         CREATE TABLE kb_artifact(id INTEGER PRIMARY KEY, site TEXT NOT NULL, kind TEXT NOT NULL, path TEXT NOT NULL, sha256 TEXT NOT NULL, bytes INTEGER NOT NULL, UNIQUE(site,kind,path));",
    ],
};

/// The knowledge base: reads and writes.
#[derive(Clone)]
pub struct Kb {
    store: Store,
}

/// The one handle the runtime gets: it records a try and the shape of an
/// answer, and reads nothing.
#[derive(Clone)]
pub struct Recorder {
    store: Store,
}

/// Where a try came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Setup,
    Check,
    Refresh,
    #[default]
    Call,
    /// A read an agent's preparation loop (`itda_prepare`) tried on a draft
    /// before anything was installed. Kept as a record, but not a try of the
    /// site's definition: the pass rate, the last success and the rules
    /// leave it out.
    Prepare,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Check => "check",
            Self::Refresh => "refresh",
            Self::Call => "call",
            Self::Prepare => "prepare",
        }
    }
}

/// The scope of the notes a preparation loop leaves: evidence and verdicts
/// about a draft, which the rules do not hold against the definition.
pub const PREPARE_SCOPE: &str = "prepare";

/// One try of an action: what happened, never what was sent or received.
#[derive(Clone, Debug, Default)]
pub struct Attempt {
    pub site: String,
    pub action: String,
    pub stage: Stage,
    /// `ok`, or the error code the call ended with.
    pub verdict: String,
    pub http_status: Option<u16>,
    pub duration_ms: Option<i64>,
    /// A fixed word for why, such as the reason a read was treated as a
    /// change; never the site's words.
    pub reason: String,
    /// The shape of the answer (keys and types), when the try answered.
    pub shape: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: i64,
    pub site: String,
    pub kind: String,
    pub key: String,
    pub props: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub src: i64,
    pub dst: i64,
    pub rel: String,
    pub props: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub id: i64,
    pub node: i64,
    pub statement: String,
    pub scope: String,
    pub confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub kind: String,
    pub path: String,
    pub sha256: String,
    pub bytes: i64,
}

/// A problem the rules found, with what to do about it: the next command
/// to run, or the next tool to call with its arguments.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Gap {
    pub rule: String,
    pub subject: String,
    /// 1 is worth looking at first.
    pub priority: u8,
    pub reason: String,
    pub remedy: Remedy,
    /// The parts the reason is made of, for a sentence in another language.
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Remedy {
    pub command: String,
    pub tool: String,
    pub arguments: Value,
}

/// How an action has fared recently.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActionStats {
    pub action: String,
    /// Tries looked at: the last [`RECENT`] at most.
    pub attempts: usize,
    pub passed: usize,
    pub pass_rate: f64,
    /// The last try answered as its definition says.
    pub verified: bool,
    pub last_verdict: Option<String>,
    pub last_ok_at: Option<String>,
    /// Two or more failures among the last [`FLAP_WINDOW`] tries that also
    /// hold a pass.
    pub unstable: bool,
}

/// A site at a glance, from its actions' stats.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    /// Actions whose last try answered as their definition says.
    pub verified: usize,
    /// The share of recent tries that answered, across the actions; `None`
    /// before the first try.
    pub pass_rate: Option<f64>,
    /// The reads that fail now and then.
    pub unstable: Vec<String>,
}

/// Sum up the stats of a site's actions.
pub fn summary(stats: &[ActionStats]) -> Summary {
    let tries: usize = stats.iter().map(|s| s.attempts).sum();
    let passed: usize = stats.iter().map(|s| s.passed).sum();
    Summary {
        verified: stats.iter().filter(|s| s.verified).count(),
        pass_rate: (tries > 0).then(|| passed as f64 / tries as f64),
        unstable: stats
            .iter()
            .filter(|s| s.unstable)
            .map(|s| s.action.clone())
            .collect(),
    }
}

/// How many rows a site has in each table.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counts {
    pub nodes: usize,
    pub edges: usize,
    pub attempts: usize,
    pub findings: usize,
    pub artifacts: usize,
    pub gaps: usize,
}

/// A node named by kind and key, for callers that do not hold ids.
#[derive(Clone, Copy, Debug)]
pub struct Ref<'a> {
    pub kind: &'a str,
    pub key: &'a str,
}

fn full(what: &str) -> AppError {
    AppError::new(
        itda_types::Code::Runtime,
        format!("The knowledge base holds as many {what} for this site as it keeps"),
        "Remove the site and set it up again to start its knowledge base afresh.",
    )
}

/// Props as stored: an object, without the keys set to null.
fn props_text(props: &Value) -> String {
    match props {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
        .to_string(),
        _ => "{}".into(),
    }
}

fn parse_props(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| json!({}))
}

fn upsert_node(
    conn: &Connection,
    site: &str,
    kind: &str,
    key: &str,
    props: &Value,
) -> AppResult<i64> {
    if !KINDS.contains(&kind) {
        return Err(AppError::new(
            itda_types::Code::Runtime,
            format!("{kind} is not a kind of node"),
            format!("Use one of {}.", KINDS.join(", ")),
        ));
    }
    let existing: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, props FROM kb_node WHERE site=?1 AND kind=?2 AND key=?3",
            params![site, kind, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, old)) = existing {
        let mut merged = parse_props(&old);
        // A key set to null is taken away.
        if let (Value::Object(into), Value::Object(from)) = (&mut merged, props) {
            for (k, v) in from {
                if v.is_null() {
                    into.remove(k);
                } else {
                    into.insert(k.clone(), v.clone());
                }
            }
        }
        conn.execute(
            "UPDATE kb_node SET props=?2 WHERE id=?1",
            params![id, props_text(&merged)],
        )?;
        return Ok(id);
    }
    let count: i64 = conn.query_row("SELECT count(*) FROM kb_node WHERE site=?1", [site], |r| {
        r.get(0)
    })?;
    if count as usize >= NODES_PER_SITE {
        return Err(full("nodes"));
    }
    conn.execute(
        "INSERT INTO kb_node(site,kind,key,props) VALUES(?1,?2,?3,?4)",
        params![site, kind, key, props_text(props)],
    )?;
    Ok(conn.last_insert_rowid())
}

fn node_id(conn: &Connection, site: &str, node: Ref<'_>) -> AppResult<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM kb_node WHERE site=?1 AND kind=?2 AND key=?3",
            params![site, node.kind, node.key],
            |r| r.get(0),
        )
        .optional()?)
}

fn record_attempt(conn: &Connection, attempt: &Attempt) -> AppResult<()> {
    let node = upsert_node(conn, &attempt.site, "action", &attempt.action, &json!({}))?;
    conn.execute(
        "INSERT INTO kb_attempt(node,stage,action,verdict,http_status,duration_ms,at,reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            node,
            attempt.stage.as_str(),
            attempt.action,
            attempt.verdict,
            attempt.http_status.map(i64::from),
            attempt.duration_ms,
            itda_types::util::now(),
            attempt.reason,
        ],
    )?;
    // Tries and refused calls are kept apart, so a run of refusals never
    // pushes the tries out.
    for (kind, keep) in [
        (sent(), ATTEMPTS_PER_ACTION),
        (format!("NOT ({})", sent()), NOT_SENT_PER_ACTION),
    ] {
        conn.execute(
            &format!(
                "DELETE FROM kb_attempt WHERE node=?1 AND {kind} AND id NOT IN (SELECT id FROM kb_attempt WHERE node=?1 AND {kind} ORDER BY id DESC LIMIT ?2)"
            ),
            params![node, keep as i64],
        )?;
    }
    if let Some(shape) = &attempt.shape {
        upsert_node(
            conn,
            &attempt.site,
            "action",
            &attempt.action,
            &json!({"shape": shape}),
        )?;
    }
    Ok(())
}

impl Recorder {
    /// Record one try. The runtime calls this after every action, whatever
    /// the outcome.
    pub async fn attempt(&self, attempt: Attempt) -> AppResult<()> {
        self.store
            .call(move |conn| record_attempt(conn, &attempt))
            .await
    }
}

fn read_nodes(
    conn: &Connection,
    site: &str,
    kind: Option<&str>,
    key: Option<&str>,
) -> AppResult<Vec<Node>> {
    let mut rows = Vec::new();
    let mut statement = conn.prepare(
        "SELECT id, site, kind, key, props FROM kb_node WHERE site=?1 AND (?2 IS NULL OR kind=?2) AND (?3 IS NULL OR key=?3) ORDER BY id",
    )?;
    let found = statement.query_map(params![site, kind, key], |r| {
        Ok(Node {
            id: r.get(0)?,
            site: r.get(1)?,
            kind: r.get(2)?,
            key: r.get(3)?,
            props: parse_props(&r.get::<_, String>(4)?),
        })
    })?;
    for node in found {
        rows.push(node?);
    }
    Ok(rows)
}

fn read_edges(conn: &Connection, site: &str) -> AppResult<Vec<Edge>> {
    let mut statement = conn.prepare(
        "SELECT e.src, e.dst, e.rel, e.props FROM kb_edge e WHERE e.src IN (SELECT id FROM kb_node WHERE site=?1) OR e.dst IN (SELECT id FROM kb_node WHERE site=?1) ORDER BY e.src, e.dst, e.rel",
    )?;
    let found = statement.query_map([site], |r| {
        Ok(Edge {
            src: r.get(0)?,
            dst: r.get(1)?,
            rel: r.get(2)?,
            props: parse_props(&r.get::<_, String>(3)?),
        })
    })?;
    let mut rows = Vec::new();
    for edge in found {
        rows.push(edge?);
    }
    Ok(rows)
}

/// A try as the statistics read it.
struct Try {
    verdict: String,
    at: String,
}

/// The last [`RECENT`] tries of a node that went to the site, newest first.
/// A preparation loop's tries of a draft are not tries of the definition.
fn recent_tries(conn: &Connection, node: i64) -> AppResult<Vec<Try>> {
    let mut statement = conn.prepare(&format!(
        "SELECT verdict, at FROM kb_attempt WHERE node=?1 AND {} AND stage<>'prepare' ORDER BY id DESC LIMIT ?2",
        sent()
    ))?;
    let found = statement.query_map(params![node, RECENT as i64], |r| {
        Ok(Try {
            verdict: r.get(0)?,
            at: r.get(1)?,
        })
    })?;
    let mut rows = Vec::new();
    for t in found {
        rows.push(t?);
    }
    Ok(rows)
}

/// Whether a node stands for an action of the definition as it is now: not
/// the history of a changed one (`name@<hash>`), not one a refresh or the
/// setup took out.
fn current(node: &Node) -> bool {
    node.kind == "action"
        && !node.key.contains('@')
        && node.props.get("removed").and_then(Value::as_bool) != Some(true)
}

fn stats_of(action: &str, tries: &[Try]) -> ActionStats {
    let passed = tries.iter().filter(|t| t.verdict == "ok").count();
    let window = &tries[..tries.len().min(FLAP_WINDOW)];
    let failed_recently = window.iter().filter(|t| t.verdict != "ok").count();
    let passed_recently = window.iter().any(|t| t.verdict == "ok");
    ActionStats {
        action: action.to_owned(),
        attempts: tries.len(),
        passed,
        pass_rate: if tries.is_empty() {
            0.0
        } else {
            passed as f64 / tries.len() as f64
        },
        verified: tries.first().is_some_and(|t| t.verdict == "ok"),
        last_verdict: tries.first().map(|t| t.verdict.clone()),
        last_ok_at: tries
            .iter()
            .find(|t| t.verdict == "ok")
            .map(|t| t.at.clone()),
        unstable: failed_recently >= 2 && passed_recently,
    }
}

fn read_stats(conn: &Connection, site: &str) -> AppResult<Vec<ActionStats>> {
    let mut out = Vec::new();
    for node in read_nodes(conn, site, Some("action"), None)? {
        if !current(&node) {
            continue;
        }
        let tries = recent_tries(conn, node.id)?;
        out.push(stats_of(&node.key, &tries));
    }
    Ok(out)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn remedy_command(site: &str, command: &str) -> Remedy {
    let (tool, arguments) = match command.split(' ').nth(1) {
        Some("check") => ("itda_check", json!({"site": site})),
        Some("refresh") => ("itda_manage", json!({"action": "refresh", "site": site})),
        Some("describe") => ("itda_sites", json!({"site": site})),
        Some("fsck") => ("itda_manage", json!({"action": "fsck", "site": site})),
        // The action's own tool; a server listing the catalog instead
        // turns this into `itda_call`.
        Some("call") => {
            let target = command.split(' ').nth(2).unwrap_or_default();
            let action = target.split_once('.').map_or(target, |(_, a)| a);
            return Remedy {
                command: command.to_owned(),
                tool: format!("{site}.{action}"),
                arguments: json!({}),
            };
        }
        _ => ("itda_sites", json!({"site": site})),
    };
    Remedy {
        command: command.to_owned(),
        tool: tool.to_owned(),
        arguments,
    }
}

fn gap(
    rule: &str,
    subject: &str,
    priority: u8,
    reason: String,
    remedy: Remedy,
    fields: &[(&str, &str)],
) -> Gap {
    Gap {
        rule: rule.to_owned(),
        subject: subject.to_owned(),
        priority,
        reason,
        remedy,
        fields: fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

/// The deterministic rules, over this site's rows. `artifacts` says what is
/// on disk now: the hash of each recorded path, or `None` when it is gone.
fn compute_gaps(
    conn: &Connection,
    site: &str,
    artifacts: &[(String, Option<String>)],
) -> AppResult<Vec<Gap>> {
    let nodes = read_nodes(conn, site, None, None)?;
    let edges = read_edges(conn, site)?;
    let mut gaps = Vec::new();
    let by_id: BTreeMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let check = remedy_command(site, &format!("itda check {site}"));
    let refresh = remedy_command(site, &format!("itda refresh {site}"));

    // One gap per action from the first three rules: a read that needs an
    // input says so, and is not also reported as never tried. A read tried
    // and failing is `read_failing` or `flapping_read` instead.
    let mut reported: Vec<i64> = Vec::new();
    for node in nodes.iter().filter(|n| current(n)) {
        let tries = recent_tries(conn, node.id)?;
        let needs = strings(node.props.get("needs"));
        let effect = node
            .props
            .get("effect")
            .and_then(Value::as_str)
            .unwrap_or("read");
        let declared = node.props.get("method").is_some();
        let describe = remedy_command(site, &format!("itda describe {site}.{}", node.key));
        if effect == "read" && !needs.is_empty() && tries.is_empty() {
            let fed = edges
                .iter()
                .filter(|e| e.rel == "feeds_input" && e.dst == node.id)
                .find_map(|e| {
                    let from = by_id.get(&e.src)?;
                    let pointer = e.props.get("pointer")?.as_str()?;
                    let input = e.props.get("input")?.as_str()?;
                    Some((from.key.clone(), pointer.to_owned(), input.to_owned()))
                });
            let inputs = needs.join(", ");
            reported.push(node.id);
            match fed {
                Some((from, pointer, input)) => gaps.push(gap(
                    "read_needs_input",
                    &node.key,
                    2,
                    format!(
                        "{} needs {input}; {from}'s {pointer} can supply it",
                        node.key
                    ),
                    remedy_command(site, &format!("itda call {site}.{from} --json")),
                    &[
                        ("action", &node.key),
                        ("inputs", &inputs),
                        ("input", &input),
                        ("from", &from),
                        ("pointer", &pointer),
                    ],
                )),
                None => gaps.push(gap(
                    "read_needs_input",
                    &node.key,
                    2,
                    format!("{} needs {inputs}", node.key),
                    describe,
                    &[("action", &node.key), ("inputs", &inputs)],
                )),
            }
        } else if declared && tries.is_empty() {
            // A read nobody ran comes before a change nobody asked for:
            // itda never tries a change on its own. A check skips a read
            // that saves a file, so that one is called by name.
            let saves_file = node.props.get("saves_file").and_then(Value::as_bool) == Some(true);
            let (priority, remedy) = match effect {
                "read" if !saves_file => (2, check.clone()),
                "read" => (2, describe),
                _ => (3, describe),
            };
            reported.push(node.id);
            gaps.push(gap(
                "endpoint_untried",
                &node.key,
                priority,
                format!("{} has never been tried", node.key),
                remedy,
                &[("action", &node.key)],
            ));
        }
        let stats = stats_of(&node.key, &tries);
        let window = tries.len().min(FLAP_WINDOW);
        if effect == "read" && window > 0 && !tries[..window].iter().any(|t| t.verdict == "ok") {
            gaps.push(gap(
                "read_failing",
                &node.key,
                1,
                format!("{} failed its last {window} tries", node.key),
                check.clone(),
                &[("action", &node.key), ("m", &window.to_string())],
            ));
        }
        if effect == "read" && stats.unstable {
            let failed = tries[..window].iter().filter(|t| t.verdict != "ok").count();
            gaps.push(gap(
                "flapping_read",
                &node.key,
                1,
                format!("{} failed {failed} of the last {window} tries", node.key),
                check.clone(),
                &[
                    ("action", &node.key),
                    ("n", &failed.to_string()),
                    ("m", &window.to_string()),
                ],
            ));
        }
    }

    // A preparation loop's notes are evidence about a draft, with no try of
    // the definition behind them by design.
    let mut statement = conn.prepare(
        "SELECT f.node, f.statement FROM kb_finding f WHERE f.node IN (SELECT id FROM kb_node WHERE site=?1) AND f.scope<>'prepare' ORDER BY f.id",
    )?;
    let findings: Vec<(i64, String)> = statement
        .query_map([site], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut noted: Vec<i64> = Vec::new();
    for (node, statement) in findings {
        if reported.contains(&node) || noted.contains(&node) {
            continue;
        }
        let Some(about) = by_id.get(&node) else {
            continue;
        };
        if about.kind == "action" && !current(about) {
            continue;
        }
        if recent_tries(conn, node)?.is_empty() {
            let subject = about.key.clone();
            noted.push(node);
            let short: String = statement.chars().take(80).collect();
            gaps.push(gap(
                "finding_without_evidence",
                &subject,
                3,
                format!("a note about {subject} has no try behind it: {short}"),
                check.clone(),
                &[("subject", &subject), ("statement", &short)],
            ));
        }
    }

    let mut statement =
        conn.prepare("SELECT kind, path, sha256 FROM kb_artifact WHERE site=?1 ORDER BY id")?;
    let recorded: Vec<(String, String, String)> = statement
        .query_map([site], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    for (kind, path, sha) in recorded {
        let Some((_, now)) = artifacts.iter().find(|(p, _)| *p == path) else {
            continue;
        };
        // A definition edited by hand or a verdict file gone: a check tries
        // the reads as the definition is now and writes both down again.
        let remedy = check.clone();
        match now {
            None => gaps.push(gap(
                "artifact_missing",
                &path,
                1,
                format!("{path} is gone"),
                remedy,
                &[("path", &path), ("kind", &kind)],
            )),
            Some(hash) if *hash != sha => gaps.push(gap(
                "artifact_changed",
                &path,
                1,
                format!("{path} changed since itda last saw it"),
                remedy,
                &[("path", &path), ("kind", &kind)],
            )),
            Some(_) => {}
        }
    }

    for edge in &edges {
        if !by_id.contains_key(&edge.src) || !by_id.contains_key(&edge.dst) {
            let subject = format!("{}:{}->{}", edge.rel, edge.src, edge.dst);
            gaps.push(gap(
                "dangling_edge",
                &subject,
                3,
                format!("a link points at something itda no longer has ({subject})"),
                refresh.clone(),
                &[("subject", &subject)],
            ));
        }
    }

    gaps.sort_by(|a, b| (a.priority, &a.rule, &a.subject).cmp(&(b.priority, &b.rule, &b.subject)));
    conn.execute("DELETE FROM kb_gap WHERE site=?1", [site])?;
    for gap in &gaps {
        conn.execute(
            "INSERT INTO kb_gap(site,rule,subject,state,priority,reason) VALUES(?1,?2,?3,'open',?4,?5) ON CONFLICT(site,rule,subject) DO UPDATE SET priority=excluded.priority, reason=excluded.reason, state='open'",
            params![site, gap.rule, gap.subject, i64::from(gap.priority), gap.reason],
        )?;
    }
    Ok(gaps)
}

impl Kb {
    pub fn open(store: Store) -> AppResult<Self> {
        store.migrate(&SCHEMA)?;
        Ok(Self { store })
    }

    /// The write-only handle for the runtime.
    pub fn recorder(&self) -> Recorder {
        Recorder {
            store: self.store.clone(),
        }
    }

    /// Add or update a node; `props` is merged over what is there. Returns
    /// its id.
    pub async fn node(&self, site: &str, kind: &str, key: &str, props: Value) -> AppResult<i64> {
        let (site, kind, key) = (site.to_owned(), kind.to_owned(), key.to_owned());
        self.store
            .call(move |conn| upsert_node(conn, &site, &kind, &key, &props))
            .await
    }

    /// Rename a node, keeping its tries and edges: a refreshed action keeps
    /// its history under `name@<hash>`. When that name is taken (the same
    /// old definition changed again), `.2`, `.3` and so on are added.
    /// Returns the name it got, or `None` when there was no such node.
    pub async fn rename(&self, site: &str, node: Ref<'_>, key: &str) -> AppResult<Option<String>> {
        let (site, kind, old, new) = (
            site.to_owned(),
            node.kind.to_owned(),
            node.key.to_owned(),
            key.to_owned(),
        );
        self.store
            .call(move |conn| {
                let mut name = new.clone();
                let mut n = 1;
                while node_id(
                    conn,
                    &site,
                    Ref {
                        kind: &kind,
                        key: &name,
                    },
                )?
                .is_some()
                {
                    n += 1;
                    name = format!("{new}.{n}");
                }
                let renamed = conn.execute(
                    "UPDATE kb_node SET key=?4 WHERE site=?1 AND kind=?2 AND key=?3",
                    params![site, kind, old, name],
                )? > 0;
                Ok(renamed.then_some(name))
            })
            .await
    }

    /// Relate two nodes of `site`; both must exist.
    pub async fn edge(
        &self,
        site: &str,
        src: Ref<'_>,
        dst: Ref<'_>,
        rel: &str,
        props: Value,
    ) -> AppResult<()> {
        if !RELS.contains(&rel) {
            return Err(AppError::new(
                itda_types::Code::Runtime,
                format!("{rel} is not a relation"),
                format!("Use one of {}.", RELS.join(", ")),
            ));
        }
        let (site, rel) = (site.to_owned(), rel.to_owned());
        let (sk, skey, dk, dkey) = (
            src.kind.to_owned(),
            src.key.to_owned(),
            dst.kind.to_owned(),
            dst.key.to_owned(),
        );
        self.store
            .call(move |conn| {
                let missing = || {
                    AppError::new(
                        itda_types::Code::Runtime,
                        "An edge names a node the knowledge base does not have",
                        "Add the node first.",
                    )
                };
                let src = node_id(conn, &site, Ref { kind: &sk, key: &skey })?.ok_or_else(missing)?;
                let dst = node_id(conn, &site, Ref { kind: &dk, key: &dkey })?.ok_or_else(missing)?;
                conn.execute(
                    "INSERT INTO kb_edge(src,dst,rel,props) VALUES(?1,?2,?3,?4) ON CONFLICT(src,dst,rel) DO UPDATE SET props=excluded.props",
                    params![src, dst, rel, props_text(&props)],
                )?;
                Ok(())
            })
            .await
    }

    /// Relate two nodes by id, whether or not they exist. For tests of the
    /// `dangling_edge` rule.
    #[doc(hidden)]
    pub async fn edge_ids(&self, src: i64, dst: i64, rel: &str) -> AppResult<()> {
        let rel = rel.to_owned();
        self.store
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO kb_edge(src,dst,rel,props) VALUES(?1,?2,?3,'{}')",
                    params![src, dst, rel],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn attempt(&self, attempt: Attempt) -> AppResult<()> {
        self.recorder().attempt(attempt).await
    }

    /// Note something about a node, in words that carry no value from the
    /// site: `kept: answered with 12 records`, `removed: response_mismatch`.
    pub async fn finding(
        &self,
        site: &str,
        node: Ref<'_>,
        statement: &str,
        scope: &str,
        confidence: f64,
    ) -> AppResult<()> {
        let (site, kind, key) = (site.to_owned(), node.kind.to_owned(), node.key.to_owned());
        let (statement, scope) = (statement.to_owned(), scope.to_owned());
        self.store
            .call(move |conn| {
                // Full: the oldest notes of the site make room.
                conn.execute(
                    "DELETE FROM kb_finding WHERE id IN (SELECT id FROM kb_finding WHERE node IN (SELECT id FROM kb_node WHERE site=?1) ORDER BY id DESC LIMIT -1 OFFSET ?2)",
                    params![site, (FINDINGS_PER_SITE - 1) as i64],
                )?;
                let node = upsert_node(conn, &site, &kind, &key, &json!({}))?;
                conn.execute(
                    "INSERT INTO kb_finding(node,statement,scope,confidence) VALUES(?1,?2,?3,?4)",
                    params![node, statement, scope, confidence],
                )?;
                Ok(())
            })
            .await
    }

    /// Findings whose words match `query`, best first, in `site` or in
    /// every site.
    pub async fn search(&self, site: Option<&str>, query: &str) -> AppResult<Vec<Finding>> {
        let site = site.map(str::to_owned);
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let query = terms.join(" ");
        self.store
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT f.id, f.node, f.statement, f.scope, f.confidence FROM kb_finding_fts JOIN kb_finding f ON f.id = kb_finding_fts.rowid JOIN kb_node n ON n.id = f.node WHERE kb_finding_fts MATCH ?1 AND (?2 IS NULL OR n.site=?2) ORDER BY rank LIMIT 50",
                )?;
                let rows = statement.query_map(params![query, site], |r| {
                    Ok(Finding {
                        id: r.get(0)?,
                        node: r.get(1)?,
                        statement: r.get(2)?,
                        scope: r.get(3)?,
                        confidence: r.get(4)?,
                    })
                })?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
    }

    /// Remember the hash of a file the site is made of (its definition,
    /// its last check).
    pub async fn artifact(
        &self,
        site: &str,
        kind: &str,
        path: &str,
        sha256: &str,
        bytes: u64,
    ) -> AppResult<()> {
        let (site, kind, path, sha256) = (
            site.to_owned(),
            kind.to_owned(),
            path.to_owned(),
            sha256.to_owned(),
        );
        self.store
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO kb_artifact(site,kind,path,sha256,bytes) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(site,kind,path) DO UPDATE SET sha256=excluded.sha256, bytes=excluded.bytes",
                    params![site, kind, path, sha256, bytes as i64],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn artifacts(&self, site: &str) -> AppResult<Vec<Artifact>> {
        let site = site.to_owned();
        self.store
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT kind, path, sha256, bytes FROM kb_artifact WHERE site=?1 ORDER BY id",
                )?;
                let rows = statement.query_map([site], |r| {
                    Ok(Artifact {
                        kind: r.get(0)?,
                        path: r.get(1)?,
                        sha256: r.get(2)?,
                        bytes: r.get(3)?,
                    })
                })?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
    }

    /// The nodes of `site`, all of them or those of one kind or one key.
    pub async fn nodes(
        &self,
        site: &str,
        kind: Option<&str>,
        key: Option<&str>,
    ) -> AppResult<Vec<Node>> {
        let (site, kind, key) = (
            site.to_owned(),
            kind.map(str::to_owned),
            key.map(str::to_owned),
        );
        self.store
            .call(move |conn| read_nodes(conn, &site, kind.as_deref(), key.as_deref()))
            .await
    }

    pub async fn edges(&self, site: &str) -> AppResult<Vec<Edge>> {
        let site = site.to_owned();
        self.store.call(move |conn| read_edges(conn, &site)).await
    }

    /// How each action of `site` has fared, in node order.
    pub async fn stats(&self, site: &str) -> AppResult<Vec<ActionStats>> {
        let site = site.to_owned();
        self.store.call(move |conn| read_stats(conn, &site)).await
    }

    /// Run the rules for `site` and keep the result. `artifacts` is what
    /// is on disk now for each recorded path: its hash, or `None` when
    /// gone; a path not listed is not judged.
    pub async fn gaps(
        &self,
        site: &str,
        artifacts: Vec<(String, Option<String>)>,
    ) -> AppResult<Vec<Gap>> {
        let site = site.to_owned();
        self.store
            .call(move |conn| compute_gaps(conn, &site, &artifacts))
            .await
    }

    /// How many rows the site has.
    pub async fn counts(&self, site: &str) -> AppResult<Counts> {
        let site = site.to_owned();
        self.store
            .call(move |conn| {
                let count = |sql: &str| -> AppResult<usize> {
                    Ok(conn.query_row(sql, [&site], |r| r.get::<_, i64>(0))? as usize)
                };
                Ok(Counts {
                    nodes: count("SELECT count(*) FROM kb_node WHERE site=?1")?,
                    edges: count("SELECT count(*) FROM kb_edge WHERE src IN (SELECT id FROM kb_node WHERE site=?1)")?,
                    attempts: count("SELECT count(*) FROM kb_attempt WHERE node IN (SELECT id FROM kb_node WHERE site=?1)")?,
                    findings: count("SELECT count(*) FROM kb_finding WHERE node IN (SELECT id FROM kb_node WHERE site=?1)")?,
                    artifacts: count("SELECT count(*) FROM kb_artifact WHERE site=?1")?,
                    gaps: count("SELECT count(*) FROM kb_gap WHERE site=?1")?,
                })
            })
            .await
    }

    /// Every site that has rows.
    pub async fn sites(&self) -> AppResult<Vec<String>> {
        self.store
            .call(|conn| {
                let mut statement =
                    conn.prepare("SELECT DISTINCT site FROM kb_node ORDER BY site")?;
                let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
    }

    /// Forget everything about `site`. `itda remove` calls this.
    pub async fn forget(&self, site: &str) -> AppResult<()> {
        let site = site.to_owned();
        self.store
            .call(move |conn| {
                let tx = conn.unchecked_transaction()?;
                tx.execute("DELETE FROM kb_edge WHERE src IN (SELECT id FROM kb_node WHERE site=?1) OR dst IN (SELECT id FROM kb_node WHERE site=?1)", [&site])?;
                tx.execute("DELETE FROM kb_attempt WHERE node IN (SELECT id FROM kb_node WHERE site=?1)", [&site])?;
                tx.execute("DELETE FROM kb_finding WHERE node IN (SELECT id FROM kb_node WHERE site=?1)", [&site])?;
                tx.execute("DELETE FROM kb_node WHERE site=?1", [&site])?;
                tx.execute("DELETE FROM kb_gap WHERE site=?1", [&site])?;
                tx.execute("DELETE FROM kb_artifact WHERE site=?1", [&site])?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Every row of every table as text, one per line. For tests that
    /// look for values that must not be there.
    pub async fn dump(&self) -> AppResult<String> {
        self.store
            .call(|conn| {
                let mut out = String::new();
                for table in [
                    "kb_node",
                    "kb_edge",
                    "kb_attempt",
                    "kb_gap",
                    "kb_finding",
                    "kb_artifact",
                ] {
                    let mut statement = conn.prepare(&format!("SELECT * FROM {table}"))?;
                    let columns = statement.column_count();
                    let rows = statement.query_map([], |r| {
                        let mut line = table.to_owned();
                        for i in 0..columns {
                            let cell: rusqlite::types::Value = r.get(i)?;
                            let text = match cell {
                                rusqlite::types::Value::Null => "null".to_owned(),
                                rusqlite::types::Value::Integer(n) => n.to_string(),
                                rusqlite::types::Value::Real(f) => f.to_string(),
                                rusqlite::types::Value::Text(t) => t,
                                rusqlite::types::Value::Blob(b) => format!("{} bytes", b.len()),
                            };
                            line.push('\t');
                            line.push_str(&text);
                        }
                        Ok(line)
                    })?;
                    for row in rows {
                        out.push_str(&row?);
                        out.push('\n');
                    }
                }
                Ok(out)
            })
            .await
    }
}

/// What a site's definition says about one action, as setup and check
/// write it on the action's node; never what it answered.
#[derive(Clone, Debug, Default)]
pub struct Declared<'a> {
    pub method: &'a str,
    pub path: &'a str,
    /// `read` or `write`.
    pub effect: &'a str,
    pub desc: &'a str,
    /// The inputs it cannot run without.
    pub needs: &'a [String],
    /// The read that confirms the change landed, when it names one.
    pub confirm: Option<&'a str>,
    pub saves_file: bool,
    pub turns_pages: bool,
}

impl Declared<'_> {
    /// The node's props. An action back in the definition is no longer
    /// marked removed.
    pub fn props(&self) -> Value {
        let mut props = Map::new();
        props.insert("method".into(), self.method.into());
        props.insert("path".into(), self.path.into());
        props.insert("effect".into(), self.effect.into());
        props.insert("desc".into(), self.desc.into());
        props.insert("needs".into(), json!(self.needs));
        props.insert("saves_file".into(), self.saves_file.into());
        props.insert("turns_pages".into(), self.turns_pages.into());
        props.insert("removed".into(), false.into());
        props.insert("confirm".into(), self.confirm.into());
        Value::Object(props)
    }
}
