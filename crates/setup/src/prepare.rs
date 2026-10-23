//! The preparation loop an agent runs over MCP (`itda_prepare`): itda reads
//! the site's pages and documentation for evidence, judges the definition
//! the agent drafts against it, tries its reads, and installs it the way
//! `itda setup` does. itda calls no model: the agent calling the tool is
//! the model. It sees shapes, names, paths, selectors and counts, never a
//! value the site answered with, and it speaks of evidence by id. No step
//! approves, installs without the setup path's own questions, or sends a
//! change. Sessions live in memory; evidence and verdicts go to the
//! knowledge base under the `prepare` scope, which the rules leave out.
mod judge;
mod read;

use crate::{Answers, Context, Done, Stopped};
use itda_prep::ground::Seen;
use itda_spec::Spec;
use itda_types::error::{AppError, AppResult, usage};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// Tool calls one session takes.
pub const CALLS: usize = 120;
/// Requests one session's reads of pages and documents send.
pub const PAGES: usize = 48;
/// Requests one session's checks of reads send, browser recovery included.
pub const SENDS: usize = 24;
/// Reads one `verify` runs.
pub const PER_ROUND: usize = 6;
/// Proposals after a review that found something uncovered.
pub const COVERAGE_ROUNDS: usize = 2;
/// Proposals after a check that found reads failing.
pub const REPAIR_ROUNDS: usize = 2;
/// Workflows an `understand` names.
pub const WORKFLOWS: usize = 12;
/// Self-questions a proposal carries.
pub const SELF_QA: usize = 12;
/// Sites prepared at once in one server.
const SESSIONS: usize = 8;

/// What an agent asked for.
#[derive(Clone, Debug)]
pub enum Op {
    /// Open a session for the site of `url`, or the one already open.
    Start { url: String, name: Option<String> },
    /// Read one page or document on the site, once, as evidence.
    Read { url: String },
    /// The entry page, the API description, GraphQL, the docs' routes and
    /// a draft itda makes from them, in one call; `bundles` also reads the
    /// scripts the entry page loads.
    Recon { bundles: bool },
    /// Evidence of this session and notes about the site matching words.
    Search { q: String },
    /// The site's accounts and credentials, as shapes; one read as an
    /// account; one same-site GET with its session.
    Session {
        step: String,
        account: Option<String>,
        path: Option<String>,
    },
    /// The agent's draft, judged against the evidence.
    Propose {
        understand: Option<Value>,
        spec: Value,
        self_qa: Option<Value>,
        use_recon: Option<Value>,
    },
    /// Try the kept reads.
    Verify { checks: Vec<Check> },
    /// Which kept operations cover each step of each workflow.
    Review { workflows: Value },
    /// Install the kept draft the way `itda setup` does; `replace` is what
    /// the caller's answers say when a site of that name exists.
    Install { replace: bool },
}

/// The inputs to try one read with: literals the agent chose, or
/// `$result:<action>:<pointer>` for a value an earlier read answered with.
#[derive(Clone, Debug, Default)]
pub struct Check {
    pub action: String,
    pub inputs: BTreeMap<String, String>,
}

/// One piece of feedback: what it is about, why, and the call to make next.
#[derive(Clone, Debug, Serialize)]
pub struct Feedback {
    pub kind: &'static str,
    pub subject: String,
    pub reason: String,
    pub next: Value,
}

pub(crate) fn feedback(
    kind: &'static str,
    subject: impl Into<String>,
    reason: impl Into<String>,
    next: Value,
) -> Feedback {
    Feedback {
        kind,
        subject: subject.into(),
        reason: reason.into(),
        next,
    }
}

/// What a call ends with.
pub enum Outcome {
    /// A result for the agent.
    Reply(Value),
    /// The draft was installed; `reply` carries the session's side.
    Installed { done: Box<Done>, reply: Value },
    /// The install stopped: a site of that name exists and replacing was
    /// not asked for, or no read worked.
    Stopped { stopped: Stopped, reply: Value },
}

/// One piece of evidence: what kind, where it came from, what the agent is
/// shown, and the words a search matches.
#[derive(Clone, Debug)]
pub(crate) struct Evidence {
    pub id: String,
    pub kind: &'static str,
    pub source: String,
    pub shown: Value,
    pub words: String,
}

/// A read tried, kept in memory for the `$result` chains: the answer never
/// leaves the session.
#[derive(Clone, Debug)]
pub(crate) struct Tried {
    pub answered: bool,
    pub answer: Value,
    pub record: Value,
}

/// The kept draft and the judgments it came from.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub spec: Spec,
    pub judgments: Vec<itda_prep::ground::Judgment>,
    pub no_actions: Option<String>,
}

/// One site being prepared.
pub(crate) struct Session {
    pub id: String,
    pub site: String,
    pub entry: url::Url,
    /// Hosts beyond the site's own the site linked as documentation: an
    /// API description, the servers it names.
    pub doc_hosts: BTreeSet<String>,
    pub calls: usize,
    pub pages: usize,
    pub sends: usize,
    pub evidence: Vec<Evidence>,
    /// The HTML pages read, kept for drafting (each cut at 2 MB).
    pub kept_pages: Vec<itda_prep::Page>,
    pub description: Option<itda_prep::Description>,
    pub schemas: Vec<itda_prep::graphql::Schema>,
    pub seen: Seen,
    /// What a call with the same arguments answered.
    pub done: BTreeMap<String, Value>,
    /// The draft itda made in `recon`, for `use_recon`.
    pub recon: Option<Spec>,
    pub workflows: Vec<Value>,
    pub candidate: Option<Candidate>,
    pub tried: BTreeMap<String, Tried>,
    pub proposals: usize,
    pub coverage_rounds: usize,
    pub repair_rounds: usize,
    pub reviewed: bool,
    pub failing: bool,
    /// The site refused a check: nothing more goes to it in this session.
    pub refused: bool,
    /// Why the knowledge base was not written, when it was not.
    pub unrecorded: Option<String>,
}

impl Session {
    fn new(id: String, site: String, entry: url::Url) -> Self {
        Self {
            id,
            site,
            entry,
            doc_hosts: BTreeSet::new(),
            calls: 0,
            pages: 0,
            sends: 0,
            evidence: Vec::new(),
            kept_pages: Vec::new(),
            description: None,
            schemas: Vec::new(),
            seen: Seen::default(),
            done: BTreeMap::new(),
            recon: None,
            workflows: Vec::new(),
            candidate: None,
            tried: BTreeMap::new(),
            proposals: 0,
            coverage_rounds: 0,
            repair_rounds: 0,
            reviewed: false,
            failing: false,
            refused: false,
            unrecorded: None,
        }
    }

    /// Keep a piece of evidence; its id.
    pub(crate) fn add(
        &mut self,
        kind: &'static str,
        source: &str,
        shown: Value,
        words: String,
    ) -> String {
        let id = format!("e{}", self.evidence.len() + 1);
        self.evidence.push(Evidence {
            id: id.clone(),
            kind,
            source: source.to_owned(),
            shown,
            words: format!("{kind} {source} {words}").to_lowercase(),
        });
        id
    }

    pub(crate) fn has_evidence(&self, id: &str) -> bool {
        self.evidence.iter().any(|e| e.id == id)
    }

    /// What is left of each budget.
    pub(crate) fn budgets(&self) -> Value {
        json!({
            "calls": {"used": self.calls, "limit": CALLS},
            "pages": {"used": self.pages, "limit": PAGES},
            "verify_requests": {"used": self.sends, "limit": SENDS},
            "per_verify": PER_ROUND,
            "coverage_rounds": {"used": self.coverage_rounds, "limit": COVERAGE_ROUNDS},
            "repair_rounds": {"used": self.repair_rounds, "limit": REPAIR_ROUNDS},
        })
    }
}

/// One open session, found by its id or its site while another call holds it.
struct Slot {
    id: String,
    site: String,
    session: Arc<tokio::sync::Mutex<Session>>,
}

/// The sessions one MCP server keeps, at most one per site.
#[derive(Default)]
pub struct Preparer {
    sessions: Mutex<Vec<Slot>>,
}

/// An error with a stable code, its own message and its own fix.
pub(crate) fn coded(
    code: &str,
    status: u16,
    msg: impl Into<String>,
    remedy: impl Into<String>,
) -> AppError {
    let mut error = AppError::api(code, status);
    error.msg = msg.into();
    error.remedy = remedy.into();
    error
}

pub(crate) fn exhausted(what: &str) -> AppError {
    let mut error = coded(
        "budget_exhausted",
        429,
        format!("This preparation used up its {what}"),
        "Install what is kept and verified with op install, or stop and tell the user what is missing. itda sends nothing more for this session.",
    );
    error.extra.insert("budget".into(), what.into());
    error
}

impl Preparer {
    pub fn new() -> Self {
        Self::default()
    }

    fn find(&self, id: &str) -> Option<Arc<tokio::sync::Mutex<Session>>> {
        let sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        sessions
            .iter()
            .find(|slot| slot.id == id)
            .map(|slot| slot.session.clone())
    }

    /// Run one call. `session` is the id `start` gave, for every op but
    /// `start`.
    pub async fn call(
        &self,
        ctx: &Context,
        session: Option<&str>,
        op: Op,
        answers: &mut (dyn Answers + Send),
    ) -> AppResult<Outcome> {
        if let Op::Start { url, name } = &op {
            return self.start(ctx, url, name.as_deref()).await;
        }
        let Some(id) = session else {
            return Err(usage(
                "session is required",
                "Call op start with the site's address first and pass the session it returns.",
            ));
        };
        let Some(slot) = self.find(id) else {
            return Err(coded(
                "session_unknown",
                404,
                format!("No preparation session {id}"),
                "Call op start with the site's address; sessions last as long as this server.",
            ));
        };
        let mut session = slot.lock().await;
        session.calls += 1;
        if session.calls > CALLS {
            return Err(exhausted("tool calls"));
        }
        match op {
            Op::Start { .. } => Err(AppError::api("internal", 500)),
            Op::Read { url } => read::read(ctx, &mut session, &url)
                .await
                .map(Outcome::Reply),
            Op::Recon { bundles } => read::recon(ctx, &mut session, bundles)
                .await
                .map(Outcome::Reply),
            Op::Search { q } => read::search(ctx, &session, &q).await.map(Outcome::Reply),
            Op::Session {
                step,
                account,
                path,
            } => judge::session(
                ctx,
                &mut session,
                &step,
                account.as_deref(),
                path.as_deref(),
            )
            .await
            .map(Outcome::Reply),
            Op::Propose {
                understand,
                spec,
                self_qa,
                use_recon,
            } => judge::propose(ctx, &mut session, understand, spec, self_qa, use_recon)
                .await
                .map(Outcome::Reply),
            Op::Verify { checks } => judge::verify(ctx, &mut session, checks)
                .await
                .map(Outcome::Reply),
            Op::Review { workflows } => judge::review(&mut session, &workflows).map(Outcome::Reply),
            Op::Install { .. } => judge::install(ctx, &mut session, answers).await,
        }
    }

    async fn start(&self, ctx: &Context, url: &str, name: Option<&str>) -> AppResult<Outcome> {
        let entry = itda_types::url::website_url(url)?;
        let host = entry.host_str().unwrap_or_default().to_owned();
        let site = name
            .map(itda_prep::slug)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| itda_prep::site_name(&host));
        if !itda_types::util::valid_id(&site) {
            return Err(usage(
                format!("{site} is not a site id"),
                "Pass name as a short lower-case id.",
            ));
        }
        let existing = {
            let sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            sessions
                .iter()
                .find(|slot| slot.site == site)
                .map(|slot| slot.session.clone())
        };
        let (slot, again) = match existing {
            Some(slot) => (slot, true),
            None => {
                let id = format!("p-{}", &itda_types::util::id()[..8]);
                let slot = Arc::new(tokio::sync::Mutex::new(Session::new(
                    id.clone(),
                    site.clone(),
                    entry.clone(),
                )));
                let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                if sessions.len() >= SESSIONS {
                    sessions.remove(0);
                }
                sessions.push(Slot {
                    id,
                    site: site.clone(),
                    session: slot.clone(),
                });
                (slot, false)
            }
        };
        let mut session = slot.lock().await;
        session.calls += 1;
        if session.calls > CALLS {
            return Err(exhausted("tool calls"));
        }
        let mut notes = Vec::new();
        if again {
            notes.push(feedback(
                "already",
                &site,
                "a session for this site is open; its evidence and budgets carry on",
                json!({"op": "recon", "session": session.id}),
            ));
        } else {
            notes.push(feedback(
                "next",
                &site,
                "read the entry page, its API description and docs in one call",
                json!({"op": "recon", "session": session.id}),
            ));
        }
        let environment = environment(ctx, &session).await;
        Ok(Outcome::Reply(json!({
            "status": "ok",
            "session": session.id,
            "site": session.site,
            "entry": crate::knowledge::page_key(session.entry.as_str()),
            "environment": environment,
            "budgets": session.budgets(),
            "feedback": notes,
        })))
    }
}

/// The browser itda would use, by name only.
fn browser_id() -> Option<&'static str> {
    if std::env::var_os("ITDA_BROWSER").is_some() {
        return Some("custom");
    }
    let path = itda_browser::find().ok()?;
    let name = path.to_string_lossy().to_ascii_lowercase();
    Some(if name.contains("edge") {
        "edge"
    } else if name.contains("brave") {
        "brave"
    } else if name.contains("chromium") {
        "chromium"
    } else {
        "chrome"
    })
}

/// What the agent works with: the browser by name, the site's accounts by
/// id and label with the names of their secrets, whether the site is paused,
/// whether it is installed. Never a path, a secret or a cookie.
async fn environment(ctx: &Context, session: &Session) -> Value {
    let origin = session.entry.origin().ascii_serialization();
    let accounts: Vec<Value> = ctx
        .vault
        .accounts(Some(&session.site))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|a| json!({"id": a.id, "label": a.label, "secrets": a.secrets}))
        .collect();
    let paused = ctx
        .net
        .gate
        .paused(&format!("{origin}|*"))
        .await
        .ok()
        .flatten();
    let installed = crate::sites::load(&ctx.home, &session.site).ok();
    json!({
        "browsers": browser_id().into_iter().collect::<Vec<_>>(),
        "transport": if ctx.net.is_adaptive() { "auto" } else { "http" },
        "accounts": accounts,
        "gate": {"origin": origin, "paused": paused},
        "installed": installed.as_ref().map(|spec| json!({
            "actions": spec.operations.len(),
            "auth": spec.auth.iter().filter(|a| a.kind != "none").map(|a| a.name.clone()).collect::<Vec<_>>(),
        })),
    })
}
