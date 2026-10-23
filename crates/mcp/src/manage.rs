//! Setting a site up, signing in, checking and managing sites from inside the
//! agent's conversation, so nobody has to go back to a terminal. Every tool
//! here runs the same steps the CLI runs; the questions the CLI asks are
//! answered by the tool's arguments or by a form the client shows the person.
use crate::{Caller, Server, elicit, elicit::Confirmed, failure, structured};
use itda_policy::{Effect, operation_effect};
use itda_setup::{Answers, Context, Fallback, Finished, Stopped, check, knowledge, sites};
use itda_spec::Spec;
use itda_types::error::{AppError, usage};
use rmcp::model::{CallToolResult, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::Arc, sync::mpsc};

/// The tools this module serves, in the order they are listed.
pub(crate) const NAMES: [&str; 7] = [
    "itda_setup",
    "itda_login",
    "itda_check",
    "itda_sites",
    "itda_manage",
    "itda_shape",
    "itda_prepare",
];

/// At most this many audit entries come back from one call.
const AUDIT_LIMIT: usize = 200;

pub(crate) fn tools() -> Vec<Tool> {
    let schema = |value: Value| match value {
        Value::Object(map) => Arc::new(map),
        _ => Arc::new(Map::new()),
    };
    let read_only = ToolAnnotations::new()
        .read_only(true)
        .idempotent(true)
        .open_world(false);
    vec![
        Tool::new(
            "itda_setup",
            "Set a website up for itda from the address of a page the user uses: looks at the page as itda, drafts actions in plain words, tries every read once, keeps the ones that work, and adds the site's actions to this server's tools. A site that runs software itda knows (GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress, Gitea or Forgejo) gets that software's API from a ready-made template; the result names it under template. Otherwise, when the page shows itda nothing but a title (it is drawn by scripts), a browser window opens briefly on the user's screen so itda can watch what the page loads by itself; nothing in it is clicked or typed, and it closes on its own. Sites that need a sign-in, or still show nothing, cannot be set up here; the result says what the user should run in a terminal instead.",
            schema(json!({"type": "object", "properties": {
                "url": {"type": "string", "description": "The address of a page on the site."},
                "name": {"type": "string", "description": "Site id to save under; taken from the host name when left out."},
                "replace": {"type": "boolean", "description": "Replace a site already set up under that name. Off unless the user asked for it."},
                "template": {"type": "string", "description": "Force a software template (gitlab, jira, confluence, redmine, discourse, mediawiki, wordpress, gitea) whatever the page says, or \"none\" to read the page like any other. Omit to let itda recognise the software from the page."}
            }, "required": ["url"], "additionalProperties": false})),
        )
        .with_annotations(ToolAnnotations::new().read_only(false).open_world(true)),
        Tool::new(
            "itda_login",
            "Open the user's browser to sign in to a site already set up. itda keeps the session for that site; agents only see the account id. Call when a read returns auth_required or itda_check says signed_out. Returns when the sign-in cookie named in the definition appears.",
            schema(json!({"type": "object", "properties": {
                "site": {"type": "string"},
                "account": {"type": "string", "description": "Account id to keep the session as; defaults to \"default\"."},
                "page": {"type": "string", "description": "The sign-in page to open, on the same site; defaults to the site's address."}
            }, "required": ["site"], "additionalProperties": false})),
        )
        .with_annotations(ToolAnnotations::new().read_only(false).open_world(true)),
        Tool::new(
            "itda_check",
            "Run all reads needing no input for one site or all. Report each site's status (ok, signed_out, site_changed, blocked or unreachable) and what to do. Never sends a change. Call before retrying an action that used to work.",
            schema(json!({"type": "object", "properties": {
                "site": {"type": "string", "description": "One site; every site when left out."},
                "account": {"type": "string"}
            }, "additionalProperties": false})),
        )
        .with_annotations(ToolAnnotations::new().read_only(true).open_world(true)),
        Tool::new(
            "itda_sites",
            "List the sites set up on this computer: name, title, how many reads and changes, the verdict of the last check with when it ran, how many actions answered on their last try (verified), the recent pass rate and the reads that fail now and then (unstable). With site, one site in detail: every action with its verified flag and pass rate, a user_guide (what can be read, what can be changed, which list read supplies which detail read's input), natural_prompts a person might ask, and the top gaps with their remedy (the next tool call).",
            schema(json!({"type": "object", "properties": {
                "site": {"type": "string", "description": "One site, in detail; every site in brief when left out."}
            }, "additionalProperties": false})),
        )
        .with_annotations(read_only.clone()),
        Tool::new(
            "itda_manage",
            "Manage itda without a terminal. refresh: when itda_check says site_changed, draft the site again from the sources it was set up from, show the user what would change (actions added, removed, changed; the ones they edited by hand are kept, as are the sign-in and the site's name) in a form they confirm, write it and check the site. remove: forget a site (the user confirms in a form; its accounts stay). resume: lift the pause after a site blocked itda (the user confirms, after checking the site in a browser). approvals: the changes waiting for the user's decision. audit: the last entries of the audit log, for one site or all, plus unknown_outcome: changes interrupted mid-process and changes whose answer was lost, each with what the confirming read said (landed, not_found or unknown) when the change named one. fsck: what the local knowledge base's rules find worth a look, for one site or all, without sending anything: endpoint_untried (an action nobody ran), read_needs_input (a read that needs an input, and which list read's pointer can supply it), read_failing (failed every recent try), flapping_read (fails now and then), artifact_changed or artifact_missing (the definition or the last check changed under itda), finding_without_evidence, dangling_edge. Each gap carries a remedy: the command, and the tool and arguments to call next. Ask it to learn in one call what has not been tried, what needs an input and what fails often.",
            schema(json!({"type": "object", "properties": {
                "action": {"type": "string", "enum": ["refresh", "remove", "resume", "approvals", "audit", "fsck"]},
                "site": {"type": "string", "description": "Needed by refresh, remove and resume; narrows audit."},
                "account": {"type": "string", "description": "resume: only this account's pause."},
                "last": {"type": "integer", "description": "audit: how many entries, 20 unless given, 200 at most."}
            }, "required": ["action"], "additionalProperties": false})),
        )
        .with_annotations(ToolAnnotations::new().read_only(false).open_world(false)),
        Tool::new(
            "itda_shape",
            "The shape of what a site shows and answers, without any values: for a page, its selectors with match counts and attribute names (structure), its named fields without values (fields) and the keys and types of the data it embeds (ssr_payloads); for an action, the keys and types of its last answer (shape) and what its definition declares. By default from the local knowledge base, written when the site was set up, checked or called. With fresh set to true, itda reads the page now (the site's address, or a page on the same site) the way setup reads a page, and records the shape. Never values, records, inputs, cookies or headers.",
            schema(json!({"type": "object", "properties": {
                "site": {"type": "string"},
                "action": {"type": "string", "description": "One action's shape."},
                "page": {"type": "string", "description": "One page's shape, by address; the site's own address when neither action nor page is given."},
                "fresh": {"type": "boolean", "description": "Fetch the page now, once, instead of reading what is known. Off unless asked."}
            }, "required": ["site"], "additionalProperties": false})),
        )
        .with_annotations(ToolAnnotations::new().read_only(true).idempotent(true).open_world(true)),
        crate::prepare::tool(),
    ]
}

/// The answers an agent's tool call gives: never record, never keep a
/// credential, replace only when asked to.
pub(crate) struct Fixed {
    pub(crate) replace: bool,
}

impl Answers for Fixed {
    fn record_anyway(&mut self) -> itda_types::error::AppResult<bool> {
        Ok(false)
    }

    /// A password never comes through an agent.
    fn sign_in(
        &mut self,
        _host: &str,
    ) -> itda_types::error::AppResult<Option<itda_setup::login::Credentials>> {
        Ok(None)
    }

    fn record(&mut self, _why: Fallback) -> itda_types::error::AppResult<bool> {
        Ok(false)
    }

    fn recording_done(&mut self) -> Option<mpsc::Receiver<()>> {
        None
    }

    fn replace(&mut self, _site: &str) -> itda_types::error::AppResult<bool> {
        Ok(self.replace)
    }

    fn can_keep_credentials(&self) -> bool {
        false
    }

    fn keep_credential(&mut self, _site: &str, _what: &str) -> itda_types::error::AppResult<bool> {
        Ok(false)
    }
}

impl Server {
    /// Run one of this module's tools; `None` when `name` is not one of
    /// them, or when the server was started without a setup context.
    pub(crate) async fn manage(
        &self,
        name: &str,
        arguments: &Map<String, Value>,
        caller: Option<&Caller<'_>>,
    ) -> Option<CallToolResult> {
        let ctx = self.setup.as_ref()?;
        if let Some(only) = self.site()
            && let Some(error) = outside(name, only, arguments)
        {
            return Some(failure(&error));
        }
        Some(match name {
            "itda_setup" => self.setup_site(ctx, arguments, caller).await,
            "itda_login" => login(ctx, arguments, caller).await,
            "itda_check" => check_sites(ctx, self.site(), arguments).await,
            "itda_sites" => match text(arguments, "site") {
                Some(site) => self.site_detail(ctx, site).await,
                None => list_sites(ctx, self.site()).await,
            },
            "itda_manage" => self.manage_action(ctx, arguments, caller).await,
            "itda_shape" => shape(ctx, arguments).await,
            "itda_prepare" => self.prepare(ctx, arguments, caller).await,
            _ => return None,
        })
    }

    async fn setup_site(
        &self,
        ctx: &Context,
        arguments: &Map<String, Value>,
        caller: Option<&Caller<'_>>,
    ) -> CallToolResult {
        let Some(url) = text(arguments, "url") else {
            return failure(&usage("url is required", "Pass the address of a page."));
        };
        let mut fixed = Fixed {
            replace: arguments
                .get("replace")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        let mut notes = Vec::new();
        let mut watched = None;
        let request = itda_setup::Request {
            url: url.to_owned(),
            name: text(arguments, "name").map(str::to_owned),
            record: false,
            template: match text(arguments, "template") {
                Some("none") => itda_setup::Template::Off,
                Some(name) => itda_setup::Template::Named(name.to_owned()),
                None => itda_setup::Template::Auto,
            },
            watch: true,
        };
        let finished = itda_setup::run(ctx, request, &mut fixed, &mut |progress| match progress {
            itda_setup::Progress::Recognised(found) => notes.push(if found.confirmed {
                format!(
                    "This looks like {}. Using its API at {}.",
                    found.title, found.base
                )
            } else {
                format!(
                    "Using the {} template at {} as asked; its API did not answer there yet.",
                    found.title, found.base
                )
            }),
            itda_setup::Progress::Bundled(known) => {
                notes.push(format!("This is {}. Using its {}.", known.title, known.api));
            }
            itda_setup::Progress::SchemaAsked { url } => {
                notes.push(format!("Asked the GraphQL API at {url} what it offers."));
            }
            itda_setup::Progress::Described(described) => notes.push(format!(
                "Read its API description at {}: {} reads, {} changes noted.",
                described.url, described.reads, described.changes
            )),
            itda_setup::Progress::Watched { calls, reads } => {
                watched = Some(watched_words(calls, reads));
            }
            _ => {}
        })
        .await;
        notes.extend(watched.clone());
        let done = match finished {
            Err(error) => return failure(&error),
            Ok(Finished::Done(done)) => done,
            Ok(Finished::Stopped(Stopped::NoRecord { why, refused })) => {
                let why = match (&refused, why) {
                    (Some(refused), _) => refusal_words(refused),
                    (None, Fallback::Blocked) => "the site refused".to_owned(),
                    (None, Fallback::SignIn) => "the page needs a sign-in".to_owned(),
                    (None, Fallback::Scripted) => "the page is drawn by scripts".to_owned(),
                };
                let seen = watched.map(|w| format!(" {w}")).unwrap_or_default();
                return failure(&coded(
                    "needs_recording",
                    422,
                    format!("itda could not read the page on its own ({why}).{seen}"),
                    format!(
                        "Ask the user to run `itda setup {url}` in a terminal and accept the offer to open their browser and record, or run `itda record {url} --install`."
                    ),
                ));
            }
            Ok(Finished::Stopped(Stopped::Exists(site))) => {
                return failure(&coded(
                    "site_exists",
                    409,
                    format!("'{site}' is already set up"),
                    "Call again with replace set to true if the user wants it replaced, or with another name.",
                ));
            }
            Ok(Finished::Stopped(Stopped::NothingWorks)) => {
                let mut error = coded(
                    "nothing_works",
                    422,
                    "No read worked, so itda kept nothing",
                    format!(
                        "Prepare the site from its docs and pages with itda_prepare (op start, url {url}), or ask the user to run `itda record {url} --install` in a terminal and use the site while it watches."
                    ),
                );
                error.extra.insert(
                    "next".into(),
                    json!({"tool": "itda_prepare", "arguments": {"op": "start", "url": url}}),
                );
                return failure(&error);
            }
        };
        if let Err(error) = self.reload(ctx) {
            return failure(&error);
        }
        self.tools_changed(caller).await;
        let mut result = self.done_json(&done, notes);
        // Little that answers: the agent can prepare the site from its docs
        // and pages instead, as a loop it runs.
        let answered = done.outcomes.iter().filter(|o| o.status == "ok").count();
        if answered <= 1 {
            result["remedy"] = json!({
                "reason": format!("only {answered} read answered; an agent can draft the site from its docs and pages with itda_prepare"),
                "tool": "itda_prepare",
                "arguments": {"op": "start", "url": url},
            });
        }
        structured(result)
    }

    /// A site just set up, as a tool result: its actions, what was removed
    /// and why, the notes, the problems and the top gaps.
    pub(crate) fn done_json(&self, done: &itda_setup::Done, mut notes: Vec<String>) -> Value {
        let spec = &done.spec;
        let removed: Vec<Value> = done
            .outcomes
            .iter()
            .filter(|o| done.removed.contains(&o.action))
            .map(|o| json!({"action": o.action, "status": o.status, "why": o.detail}))
            .collect();
        notes.extend(done.notes.iter().cloned());
        notes.extend(done.unrecorded.as_deref().map(knowledge::not_written));
        json!({
            "status": "ok",
            "site": spec.site.name,
            "title": spec.site.title,
            "gaps": self.gaps_json(&done.gaps),
            "template": done.recognised.as_ref().map(|found| json!({
                "software": found.software,
                "title": found.title,
                "base_url": found.base,
                "version": found.version,
                "confirmed": found.confirmed,
            })),
            "actions": actions(spec),
            "removed": removed,
            "notes": notes,
            "problems": done.problems,
            "untrusted": true,
        })
    }

    async fn manage_action(
        &self,
        ctx: &Context,
        arguments: &Map<String, Value>,
        caller: Option<&Caller<'_>>,
    ) -> CallToolResult {
        let action = text(arguments, "action").unwrap_or_default();
        match action {
            "refresh" => {
                let Some(site) = text(arguments, "site") else {
                    return failure(&usage("site is required", "Name the site to refresh."));
                };
                let plan = match itda_setup::refresh::plan(ctx, site, &mut |_| {}).await {
                    Ok(plan) => plan,
                    Err(error) => return failure(&error),
                };
                let before = check::verdicts(&plan.outcomes);
                let changes = match serde_json::to_value(&plan.changes) {
                    Ok(changes) => changes,
                    Err(_) => return failure(&AppError::api("internal", 500)),
                };
                if !plan.changes_anything() {
                    return structured(json!({
                        "status": "ok",
                        "site": site,
                        "written": false,
                        "changes": changes,
                        "before": before,
                        "verdict": before.first().map(|v| v.verdict),
                        "hint": before.first().map(|v| v.hint.clone()),
                        "note": "The sources draft the same actions; itda left the definition as is.",
                        "untrusted": true,
                    }));
                }
                let mut message = format!(
                    "Refresh '{site}'? itda drafted it again from the same sources. itda keeps the sign-in, the site's name and actions you edited by hand. This would change:"
                );
                for change in &plan.changes {
                    message.push_str("\n- ");
                    message.push_str(&change_words(change));
                }
                match confirmed(caller, message, "Refresh this site").await {
                    Confirmed::Yes => {}
                    Confirmed::No => return failure(&declined("refresh", site)),
                    Confirmed::Unanswered | Confirmed::Asked => {
                        return failure(&unanswered(format!("itda refresh {site}")));
                    }
                    Confirmed::CannotAsk => {
                        return failure(&needs_terminal(format!("itda refresh {site}")));
                    }
                }
                let mut notes = plan.notes.clone();
                match itda_setup::refresh::apply(ctx, &plan).await {
                    Ok(whys) => notes.extend(whys.iter().map(|why| knowledge::not_written(why))),
                    Err(error) => return failure(&error),
                }
                if let Err(error) = ctx.approvals.invalidate(Some(site), None).await {
                    return failure(&error);
                }
                if let Err(error) = self.reload(ctx) {
                    return failure(&error);
                }
                self.tools_changed(caller).await;
                let outcomes =
                    check::run(&ctx.runtime, std::slice::from_ref(&plan.spec), None).await;
                let verdicts = check::verdicts(&outcomes);
                match check::remember(
                    &ctx.home,
                    &ctx.kb,
                    std::slice::from_ref(&plan.spec),
                    &verdicts,
                )
                .await
                {
                    Ok(whys) => notes.extend(whys.iter().map(|why| knowledge::not_written(why))),
                    Err(error) => return failure(&error),
                }
                structured(json!({
                    "status": "ok",
                    "site": site,
                    "written": true,
                    "changes": changes,
                    "before": before,
                    "verdict": verdicts.first().map(|v| v.verdict),
                    "hint": verdicts.first().map(|v| v.hint.clone()),
                    "actions": actions(&plan.spec),
                    "results": outcomes,
                    "notes": notes,
                    "problems": plan.problems,
                    "untrusted": true,
                }))
            }
            "remove" => {
                let Some(site) = text(arguments, "site") else {
                    return failure(&usage("site is required", "Name the site to remove."));
                };
                if let Err(error) = sites::load(&ctx.home, site) {
                    return failure(&error);
                }
                let message = format!(
                    "Forget the site '{site}'? itda deletes its local definition. Accounts signed in to it stay until itda logout."
                );
                match confirmed(caller, message, "Remove this site").await {
                    Confirmed::Yes => {}
                    Confirmed::No => return failure(&declined("remove", site)),
                    Confirmed::Unanswered | Confirmed::Asked => {
                        return failure(&unanswered(format!("itda remove {site}")));
                    }
                    Confirmed::CannotAsk => {
                        return failure(&needs_terminal(format!("itda remove {site}")));
                    }
                }
                let spec = match sites::load(&ctx.home, site) {
                    Ok(spec) => spec,
                    Err(error) => return failure(&error),
                };
                if let Err(error) = itda_setup::reset_browser_sessions(ctx, &spec).await {
                    return failure(&error);
                }
                // What was known about the site goes first, so a failure
                // leaves the site there to remove again.
                if let Err(error) = ctx.kb.forget(site).await {
                    return failure(&error);
                }
                if let Err(error) = sites::remove(&ctx.home, site) {
                    return failure(&error);
                }
                if let Err(error) = ctx.approvals.invalidate(Some(site), None).await {
                    return failure(&error);
                }
                if let Err(error) = self.reload(ctx) {
                    return failure(&error);
                }
                self.tools_changed(caller).await;
                structured(json!({"status": "ok", "removed": site}))
            }
            "resume" => {
                let Some(site) = text(arguments, "site") else {
                    return failure(&usage("site is required", "Name the site to resume."));
                };
                if let Err(error) = sites::load(&ctx.home, site) {
                    return failure(&error);
                }
                let message = format!(
                    "Resume '{site}'? itda paused it: the site refused or rate limited itda. Check the site in your browser first; resuming lets agents call it again."
                );
                match confirmed(caller, message, "Resume this site").await {
                    Confirmed::Yes => {}
                    Confirmed::No => return failure(&declined("resume", site)),
                    Confirmed::Unanswered | Confirmed::Asked => {
                        return failure(&unanswered(format!("itda resume {site}")));
                    }
                    Confirmed::CannotAsk => {
                        return failure(&needs_terminal(format!("itda resume {site}")));
                    }
                }
                match itda_setup::resume(ctx, site, text(arguments, "account")).await {
                    Ok(()) => structured(json!({"status": "ok", "resumed": site})),
                    Err(error) => failure(&error),
                }
            }
            "approvals" => match ctx.approvals.pending().await {
                Ok(pending) => {
                    let list: Vec<Value> = pending
                        .iter()
                        .map(|a| {
                            json!({
                                "id": a.id,
                                "site": a.site_id,
                                "action": a.action_id,
                                "account": a.account_id,
                                "expires_at": a.expires_at,
                                "inputs": a.inputs.iter().map(|i| json!({"label": i.label, "value": i.value})).collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    structured(json!({"status": "ok", "approvals": list, "untrusted": true}))
                }
                Err(error) => failure(&error),
            },
            "audit" => {
                let last = arguments
                    .get("last")
                    .and_then(Value::as_u64)
                    .map_or(20, |n| n as usize)
                    .clamp(1, AUDIT_LIMIT);
                let site = text(arguments, "site");
                let records = match ctx.audit.records() {
                    Ok(records) => records,
                    Err(error) => return failure(&error),
                };
                let own: Vec<&itda_runtime::audit::Record> = records
                    .iter()
                    .filter(|r| site.is_none_or(|site| r.site == site))
                    .collect();
                let recent = &own[own.len().saturating_sub(last)..];
                let unfinished = match ctx.audit.unfinished_changes() {
                    Ok(unfinished) => unfinished,
                    Err(error) => return failure(&error),
                };
                let entry = |r: &itda_runtime::audit::Record, outcome: &str| {
                    json!({
                        "seq": r.seq,
                        "at": r.at,
                        "site": r.site,
                        "action": r.action,
                        "approval": r.approval,
                        "outcome": outcome,
                    })
                };
                let mut unknown: Vec<Value> = unfinished
                    .iter()
                    .filter(|r| site.is_none_or(|site| r.site == site))
                    .map(|r| {
                        entry(
                            r,
                            "unknown: the process stopped after the change was sent; check the site",
                        )
                    })
                    .collect();
                // Changes whose answer was lost, among the entries returned,
                // with what the confirming read said when there was one.
                unknown.extend(
                    recent
                        .iter()
                        .filter(|r| r.kind == "call" && r.outcome == "action_outcome_unknown")
                        .map(|r| {
                            let mut lost =
                                entry(r, "unknown: the answer was lost after the change was sent");
                            lost["check"] = match r.outcome_check.as_str() {
                                "" => Value::Null,
                                result => json!({"read": r.confirm_read, "result": result}),
                            };
                            lost
                        }),
                );
                match serde_json::to_value(recent) {
                    Ok(records) => structured(
                        json!({"status": "ok", "records": records, "unknown_outcome": unknown}),
                    ),
                    Err(_) => failure(&AppError::api("internal", 500)),
                }
            }
            "fsck" => {
                let specs: Vec<Spec> = match text(arguments, "site") {
                    Some(site) => match sites::load(&ctx.home, site) {
                        Ok(spec) => vec![spec],
                        Err(error) => return failure(&error),
                    },
                    None => sites::list(&ctx.home)
                        .into_iter()
                        .filter(|s| self.site().is_none_or(|only| only == s.name))
                        .filter_map(|s| s.spec.ok())
                        .collect(),
                };
                let mut list = Vec::new();
                let mut problems = false;
                for spec in &specs {
                    match knowledge::audit(&ctx.kb, &ctx.home, spec).await {
                        Ok(gaps) => {
                            problems |= gaps.iter().any(|g| g.priority == 1);
                            list.push(
                                json!({"site": spec.site.name, "gaps": self.gaps_json(&gaps)}),
                            );
                        }
                        Err(error) => return failure(&error),
                    }
                }
                structured(json!({
                    "status": if problems { "problems" } else { "ok" },
                    "sites": list,
                }))
            }
            other => failure(&usage(
                format!("Unknown action: {other}"),
                "Use refresh, remove, resume, approvals, audit or fsck.",
            )),
        }
    }

    /// Serve the sites as they are on disk now.
    pub(crate) fn reload(&self, ctx: &Context) -> itda_types::error::AppResult<()> {
        let specs: Vec<Spec> = sites::list(&ctx.home)
            .into_iter()
            .filter(|site| self.site().is_none_or(|only| only == site.name))
            .filter_map(|site| site.spec.ok())
            .collect();
        let catalog = Arc::new(crate::Catalog::new(specs));
        let mut slot = self
            .catalog
            .write()
            .map_err(|_| AppError::api("internal", 500))?;
        *slot = catalog;
        Ok(())
    }
}

async fn login(
    ctx: &Context,
    arguments: &Map<String, Value>,
    caller: Option<&Caller<'_>>,
) -> CallToolResult {
    let Some(site) = text(arguments, "site") else {
        return failure(&usage("site is required", "Name the site to sign in to."));
    };
    let account = text(arguments, "account").unwrap_or(itda_runtime::DEFAULT_ACCOUNT);
    let spec = match sites::load(&ctx.home, site) {
        Ok(spec) => spec,
        Err(error) => return failure(&error),
    };
    if let Err(error) = itda_setup::credentials::writable() {
        return failure(&error);
    }
    let message = format!(
        "Open your browser to sign in to {site}? itda keeps the session cookies for {site} as account '{account}' and seals them under its data directory with a system keyring key. Agents only see the account id."
    );
    match confirmed(caller, message, "Open the browser").await {
        Confirmed::No => return failure(&declined("login", site)),
        // A form nobody answered is not a yes. A client that cannot ask
        // still gets the window: the person sees it open, or closes it.
        Confirmed::Unanswered | Confirmed::Asked => {
            return failure(&unanswered(format!("itda login {site} --browser")));
        }
        Confirmed::Yes | Confirmed::CannotAsk => {}
    }
    let page = text(arguments, "page").map(str::to_owned);
    let from = itda_setup::login::session_value_from(&spec)
        .unwrap_or_default()
        .to_owned();
    let session = tokio::task::spawn_blocking(move || {
        itda_setup::login::browser(&spec, page.as_deref(), None, &mut |_| {})
    })
    .await;
    let session = match session {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => return failure(&error),
        Err(_) => return failure(&AppError::api("internal", 500)),
    };
    let count = session.count;
    match itda_setup::login::keep(ctx, site, account, "", session, &from).await {
        Ok(()) => structured(json!({
            "status": "ok",
            "site": site,
            "account": account,
            "cookies": count,
            "note": "itda seals the session under the data directory with a system keyring key. Agents only see the account id.",
        })),
        Err(error) => failure(&error),
    }
}

/// What a server pinned to one site refuses: setting other sites up, and
/// naming another site anywhere.
fn outside(tool: &str, only: &str, arguments: &Map<String, Value>) -> Option<AppError> {
    let named = text(arguments, "site");
    let other = named.is_some_and(|site| site != only);
    let sets_up = matches!(tool, "itda_setup" | "itda_prepare");
    if sets_up || (tool == "itda_manage" && other) || (tool != "itda_manage" && other) {
        let what = match tool {
            "itda_setup" | "itda_prepare" => "sets other sites up".to_owned(),
            _ => format!("reaches {}", named.unwrap_or_default()),
        };
        return Some(usage(
            format!("This server serves only {only} and never {what}"),
            format!(
                "Use the tools for {only}, or ask the user to run itda in a terminal for other sites."
            ),
        ));
    }
    None
}

async fn check_sites(
    ctx: &Context,
    only: Option<&str>,
    arguments: &Map<String, Value>,
) -> CallToolResult {
    let specs = match text(arguments, "site").or(only) {
        Some(site) => match sites::load(&ctx.home, site) {
            Ok(spec) => vec![spec],
            Err(error) => return failure(&error),
        },
        None => sites::list(&ctx.home)
            .into_iter()
            .filter_map(|site| site.spec.ok())
            .collect(),
    };
    let outcomes = check::run(&ctx.runtime, &specs, text(arguments, "account")).await;
    let verdicts = check::verdicts(&outcomes);
    let notes: Vec<String> = match check::remember(&ctx.home, &ctx.kb, &specs, &verdicts).await {
        Ok(whys) => whys.iter().map(|why| knowledge::not_written(why)).collect(),
        Err(error) => return failure(&error),
    };
    let status = check::exit_status(&outcomes).to_owned();
    structured(json!({
        "status": status,
        "verdict": check::Verdict::of(&outcomes),
        "sites": verdicts,
        "results": outcomes,
        "notes": notes,
        "untrusted": true,
    }))
}

/// How a site's actions have fared, from the knowledge base: nothing when
/// it cannot be read.
async fn stats_of(ctx: &Context, spec: &Spec) -> Vec<itda_kb::ActionStats> {
    knowledge::stats(&ctx.kb, spec).await.unwrap_or_default()
}

impl Server {
    /// Gaps as a tool result carries them. A remedy that calls an action
    /// names the action's own tool; when this server lists the catalog
    /// instead, the same call goes through `itda_call`.
    fn gaps_json(&self, gaps: &[itda_kb::Gap]) -> Vec<Value> {
        let catalog = self.mode() == crate::Mode::Catalog;
        knowledge::gaps_json(gaps)
            .into_iter()
            .map(|mut gap| {
                let tool = gap["remedy"]["tool"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if catalog && tool.contains('.') {
                    gap["remedy"]["tool"] = "itda_call".into();
                    gap["remedy"]["arguments"] = json!({"action": tool, "args": {}});
                }
                gap
            })
            .collect()
    }

    /// One site in detail: its actions with how they fared, the guide, the
    /// prompts and the top gaps. English, like every tool result.
    async fn site_detail(&self, ctx: &Context, site: &str) -> CallToolResult {
        let spec = match sites::load(&ctx.home, site) {
            Ok(spec) => spec,
            Err(error) => return failure(&error),
        };
        let stats = stats_of(ctx, &spec).await;
        let stat = |name: &str| stats.iter().find(|s| s.action == name);
        let summary = itda_kb::summary(&stats);
        let flows = knowledge::flows(&ctx.kb, site).await.unwrap_or_default();
        let (guide, prompts) = knowledge::guide_for(&spec, &flows, "en");
        let mut gaps = knowledge::audit(&ctx.kb, &ctx.home, &spec)
            .await
            .unwrap_or_default();
        gaps.truncate(3);
        let mut list = actions(&spec);
        for action in &mut list {
            let name = action["action"].as_str().unwrap_or_default().to_owned();
            action["verified"] = stat(&name).is_some_and(|s| s.verified).into();
            action["pass_rate"] = json!(stat(&name).map(|s| s.pass_rate));
            action["last_verdict"] = json!(stat(&name).and_then(|s| s.last_verdict.clone()));
        }
        structured(json!({
            "status": "ok",
            "site": spec.site.name,
            "title": spec.site.title,
            "base_url": spec.site.base_url,
            "last_check": check::last(&ctx.home, site)
                .map(|last| json!({"at": last.at, "verdict": last.verdict, "hint": last.hint})),
            "verified": summary.verified,
            "pass_rate": summary.pass_rate,
            "unstable": summary.unstable,
            "actions": list,
            "user_guide": guide,
            "natural_prompts": prompts,
            "gaps": self.gaps_json(&gaps),
            "untrusted": true,
        }))
    }
}

/// An action's shape result within 24 KB: the nested `shape` goes first
/// (`pointers` say the same), then pointers from the end.
fn within(mut result: Value) -> Value {
    const MAX: usize = 24 * 1024;
    let size = |v: &Value| v.to_string().len();
    if size(&result) <= MAX {
        return result;
    }
    result["truncated"] = true.into();
    if let Some(map) = result.as_object_mut() {
        map.remove("shape");
    }
    while size(&result) > MAX {
        match result.get_mut("pointers").and_then(Value::as_array_mut) {
            Some(list) if !list.is_empty() => {
                list.pop();
            }
            _ => break,
        }
    }
    result
}

/// `itda_shape`: what is known about a page's or an action's shape, or,
/// with `fresh`, the page as it is now, read the way setup reads a page.
async fn shape(ctx: &Context, arguments: &Map<String, Value>) -> CallToolResult {
    let Some(site) = text(arguments, "site") else {
        return failure(&usage("site is required", "Name the site."));
    };
    let spec = match sites::load(&ctx.home, site) {
        Ok(spec) => spec,
        Err(error) => return failure(&error),
    };
    let fresh = arguments
        .get("fresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(action) = text(arguments, "action") {
        if fresh {
            return failure(&usage(
                "fresh fetches a page, not an action",
                "Call the action itself to get a fresh answer; its shape is recorded then.",
            ));
        }
        let op = match spec.op(action) {
            Ok(op) => op,
            Err(error) => return failure(&error),
        };
        let node = ctx
            .kb
            .nodes(site, Some("action"), Some(&op.name))
            .await
            .unwrap_or_default()
            .into_iter()
            .next();
        let shape = node.as_ref().and_then(|n| n.props.get("shape").cloned());
        return structured(within(json!({
            "status": "ok",
            "site": site,
            "action": op.name,
            "kind": "action",
            "declared": node.as_ref().map(|n| {
                let mut props = n.props.clone();
                if let Some(map) = props.as_object_mut() {
                    map.remove("shape");
                }
                props
            }),
            "pointers": shape.as_ref().map(itda_html::shape::pointers),
            "shape": shape,
            "untrusted": true,
        })));
    }
    let page = text(arguments, "page")
        .map(str::to_owned)
        .unwrap_or_else(|| spec.site.base_url.clone());
    let same_site = match (url::Url::parse(&page), url::Url::parse(&spec.site.base_url)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    };
    if !same_site {
        return failure(&usage(
            format!("{page} is not a page on {site}"),
            format!("Pass a page under {}.", spec.site.base_url),
        ));
    }
    if fresh {
        // Read the way setup reads a page, through the same gate and
        // under the same rules.
        let probed = match itda_setup::probe(ctx, &page, None).await {
            Ok(probed) => probed,
            Err(error) => return failure(&error),
        };
        let Some(html) = probed.page.as_ref().map(|p| p.html.as_str()) else {
            let why = match probed.outcome {
                Err((_, Some(refused))) => refusal_words(&refused),
                Err((Fallback::SignIn, None)) => "the page needs a sign-in".to_owned(),
                Err((Fallback::Scripted, None)) => "the page is drawn by scripts".to_owned(),
                _ => "the site refused".to_owned(),
            };
            return failure(&coded(
                "page_unavailable",
                422,
                format!("itda could not read {page} ({why})"),
                "Ask the user to check the page in a browser.",
            ));
        };
        let shaped = itda_html::shape::page(html);
        let mut props = shaped.clone();
        props["fetched"] = true.into();
        if props.get("truncated").is_none() {
            props["truncated"] = Value::Null;
        }
        let key = knowledge::page_key(&page);
        if let Err(error) = ctx.kb.node(site, "page", &key, props).await {
            return failure(&error);
        }
        return structured(json!({
            "status": "ok",
            "site": site,
            "page": key,
            "kind": "page",
            "fresh": true,
            "shape": shaped,
            "untrusted": true,
        }));
    }
    let nodes = ctx
        .kb
        .nodes(site, Some("page"), None)
        .await
        .unwrap_or_default();
    let key = knowledge::page_key(&page);
    let known = nodes.iter().find(|n| n.key == key).or_else(|| {
        // The site's own address stands for any page known on it.
        (text(arguments, "page").is_none())
            .then(|| nodes.first())
            .flatten()
    });
    match known {
        Some(node) => structured(json!({
            "status": "ok",
            "site": site,
            "page": node.key,
            "kind": "page",
            "fresh": false,
            "shape": node.props,
            "untrusted": true,
        })),
        None => failure(&coded(
            "shape_unknown",
            404,
            format!("itda has not seen {key} yet"),
            "Call again with fresh set to true to fetch it once, or name a page itda set the site up from.",
        )),
    }
}

async fn list_sites(ctx: &Context, only: Option<&str>) -> CallToolResult {
    let mut list: Vec<Value> = Vec::new();
    for site in sites::list(&ctx.home)
        .into_iter()
        .filter(|site| only.is_none_or(|only| only == site.name))
    {
        let last = check::last(&ctx.home, &site.name)
            .map(|last| json!({"at": last.at, "verdict": last.verdict, "hint": last.hint}));
        list.push(match site.spec {
            Ok(spec) => {
                let changes = spec
                    .operations
                    .iter()
                    .filter(|op| operation_effect(op, &BTreeMap::new()) != Effect::Read)
                    .count();
                let summary = itda_kb::summary(&stats_of(ctx, &spec).await);
                let mut entry = json!({
                    "site": site.name,
                    "title": spec.site.title,
                    "base_url": spec.site.base_url,
                    "reads": spec.operations.len() - changes,
                    "changes": changes,
                    "last_check": last,
                    "verified": summary.verified,
                    "pass_rate": summary.pass_rate,
                    "unstable": summary.unstable,
                });
                // When the signed-in session is a JWT the site keeps in
                // browser storage, say when it stops being accepted, so the
                // agent asks the person to sign in again before then.
                if let Some(expires) = session_expiry(ctx, &spec).await {
                    entry["session_expires"] = expires.into();
                }
                entry
            }
            Err(error) => json!({
                "site": site.name,
                "problem": error.msg,
                "remedy": error.remedy,
                "last_check": last,
            }),
        });
    }
    structured(json!({"status": "ok", "sites": list}))
}

/// When the default account's stored session stops being accepted, as a date,
/// for a site whose session is a JSON Web Token kept in browser storage.
/// `None` when the site has no such session, nobody is signed in, or the
/// token carries no expiry.
async fn session_expiry(ctx: &Context, spec: &Spec) -> Option<String> {
    let storage = spec
        .auth
        .iter()
        .any(|auth| auth.kind == "header" && auth.value_from.starts_with("storage:"));
    if !storage {
        return None;
    }
    let secret = ctx
        .vault
        .secret(
            &spec.site.name,
            itda_runtime::DEFAULT_ACCOUNT,
            itda_runtime::SESSION_SECRET,
        )
        .await
        .ok()?;
    itda_types::util::jwt_expiry(secret.expose()).map(itda_types::util::iso_date)
}

/// The actions as the agent will see them: name, effect, description and
/// the inputs it may give.
fn actions(spec: &Spec) -> Vec<Value> {
    spec.operations
        .iter()
        .map(|op| {
            let inputs: Vec<&str> = op
                .params
                .keys()
                .chain(op.query.keys())
                .chain(op.body.iter().flat_map(|b| b.inputs().map(|(n, _)| n)))
                .filter(|n| {
                    let param = op.params.get(*n).or_else(|| op.query.get(*n)).or_else(|| {
                        op.body
                            .as_ref()
                            .and_then(|b| b.inputs().find(|(k, _)| *k == *n).map(|(_, p)| p))
                    });
                    !param.is_some_and(itda_spec::Param::from_step)
                })
                .map(String::as_str)
                .filter(|n| !matches!(*n, "query" | "operationName"))
                .collect();
            let mut action = json!({
                "tool": format!("{}.{}", spec.site.name, op.name),
                "action": op.name,
                "effect": operation_effect(op, &BTreeMap::new()).as_str(),
                "description": op.desc,
                "inputs": inputs,
            });
            if op.saves_file() {
                action["returns"] = json!({"file": {"path": "string", "bytes": "integer", "media_type": "string", "sha256": "string"}});
            }
            action
        })
        .collect()
}

async fn confirmed(caller: Option<&Caller<'_>>, message: String, title: &'static str) -> Confirmed {
    let Some(caller) = caller else {
        return Confirmed::CannotAsk;
    };
    elicit::confirm(caller, message, title).await
}

fn declined(what: &str, site: &str) -> AppError {
    coded(
        "declined",
        403,
        format!("The user did not confirm the {what} for {site}"),
        "Do not ask again unless the user changes their mind.",
    )
}

fn unanswered(command: String) -> AppError {
    coded(
        "unanswered",
        408,
        "The user did not answer the confirmation form in time",
        format!(
            "Ask the user, then call again when they are ready, or ask them to run `{command}` in a terminal."
        ),
    )
}

fn needs_terminal(command: String) -> AppError {
    coded(
        "needs_confirmation",
        403,
        "This client cannot show the user a form to confirm it",
        format!("Ask the user to run `{command}` in a terminal."),
    )
}

/// One line of a refresh's diff, for the form the person confirms.
fn change_words(change: &itda_setup::refresh::Change) -> String {
    use itda_setup::refresh::Change;
    match change {
        Change::Added { name, desc } => format!("{name}: added ({desc})"),
        Change::Removed { name, why } => {
            format!("{name}: removed; it no longer drafts and its read failed with {why}")
        }
        Change::Changed { name, fields } => format!("{name}: changed ({})", fields.join(", ")),
        Change::Edited { name } => format!("{name}: kept as you edited it"),
    }
}

/// What watching the page load in a window on the user's screen gave.
fn watched_words(calls: usize, reads: usize) -> String {
    let found = if reads == 0 {
        "nothing to draft".to_owned()
    } else {
        format!("{reads} reads drafted")
    };
    format!(
        "Watched the page load in a browser window on the user's screen: {calls} calls, {found}."
    )
}

/// What the gate saw when the page refused itda, in words.
fn refusal_words(refused: &itda_setup::Refused) -> String {
    use itda_setup::Refusal;
    let host = &refused.host;
    match &refused.kind {
        Refusal::Cloudflare => format!("{host} is behind Cloudflare and refused itda"),
        Refusal::BotManager => format!("{host} runs a bot manager that refused itda"),
        Refusal::Captcha => format!("{host} asks for a captcha before it shows the page"),
        Refusal::RateLimited { wait: Some(wait) } => {
            format!("{host} limits how often it is read and asked itda to wait {wait} s")
        }
        Refusal::RateLimited { wait: None } => {
            format!("{host} limits how often it is read and refused itda for now")
        }
        Refusal::Other { status } => format!("{host} refused itda (HTTP {status})"),
        Refusal::SignInRedirect { path, .. } => {
            format!("{host} sends itda to a sign-in page instead of showing {path}")
        }
    }
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

fn text<'a>(arguments: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Send `notifications/tools/list_changed`, so the client fetches the tool
/// list again. A client that is not there to hear it loses nothing.
impl Server {
    pub(crate) async fn tools_changed(&self, caller: Option<&Caller<'_>>) {
        use rmcp::model::{NotificationNoParam, ServerNotification};
        if let Some(caller) = caller {
            let _ = caller
                .peer
                .send_notification(ServerNotification::ToolListChangedNotification(
                    NotificationNoParam {
                        method: Default::default(),
                        extensions: Default::default(),
                    },
                ))
                .await;
        }
    }
}
