//! The steps behind `itda setup`: look at a page (or record the person using
//! the site), draft actions, try every read once, keep what works. The
//! command line and the MCP server both run them; the questions along the
//! way are answered through [`Answers`] and what happens is reported through
//! [`Progress`], so each caller keeps its own words and its own way of
//! asking. Nothing here prints or reads a terminal.
//!
//! The sign-in ([`login`]), the check ([`check`]), the installed sites
//! ([`sites`]) and the credentials a recording used ([`credentials`]) are
//! the same pieces the other commands are built from. [`software`]
//! recognises the software a site runs, so a GitLab or a Jira gets its real
//! API from a ready-made template instead of a reading of the page.
pub mod check;
pub mod credentials;
pub mod description;
pub mod frames;
pub mod introspect;
pub mod login;
pub mod record;
pub mod refresh;
pub mod sites;
pub mod software;

use itda_html::evidence;
use itda_net::Network;
use itda_policy::Approvals;
use itda_prep::{Described, Draft, Options, Page};
use itda_runtime::{Runtime, audit::AuditLog};
use itda_spec::Spec;
use itda_types::error::{AppError, AppResult, usage};
use itda_vault::Vault;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

/// Everything the steps need, opened once by the composition root. The
/// runtime shares the network and the vault, so a read tried here and one an
/// agent runs later take the same path.
pub struct Context {
    pub home: PathBuf,
    pub net: Arc<Network>,
    pub vault: Arc<Vault>,
    pub approvals: Arc<Approvals>,
    pub audit: Arc<AuditLog>,
    pub runtime: Arc<Runtime>,
    /// `ITDA_LANG` or the system language: what the drafted descriptions are
    /// written in.
    pub lang: String,
}

/// What `itda setup` was asked to do.
pub struct Request {
    pub url: String,
    pub name: Option<String>,
    /// Start by recording, without looking at the page first.
    pub record: bool,
    /// Whether to use a ready-made template for software the site runs.
    pub template: Template,
    /// When the page shows itda nothing but a title and carries scripts that
    /// could fetch something, open the browser on it and watch what it loads
    /// by itself before asking anyone to record.
    pub watch: bool,
}

/// Which ready-made template to use, when the site runs software itda
/// knows (`sites/software/`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Template {
    /// Recognise the software from the page and confirm it with one request
    /// to its own API.
    #[default]
    Auto,
    /// Use this template whatever the page says (`itda setup --as jira`).
    Named(String),
    /// Read the page like any other (`--no-template`).
    Off,
}

/// Why a fetched page is not enough to draft from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    Blocked,
    SignIn,
    Scripted,
}

/// What the gate saw when a page refused itda: the block by name, a rate
/// limit with the wait it asked for, or a redirect to a sign-in page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Cloudflare,
    BotManager,
    Captcha,
    RateLimited {
        wait: Option<u64>,
    },
    Other {
        status: u16,
    },
    /// The answer sends itda to the sign-in page `to` instead of showing
    /// `path`.
    SignInRedirect {
        path: String,
        to: String,
    },
}

/// A refused page: what the gate saw, and the error `itda call` would give
/// (`blocked_by_site`, `site_rate_limited`, `auth_required`), whose words
/// the caller may replace with its own.
#[derive(Debug)]
pub struct Refused {
    /// The page's host, for the words.
    pub host: String,
    pub kind: Refusal,
    pub error: AppError,
}

impl Refused {
    /// The way the steps go on: a sign-in, or a block.
    pub fn fallback(&self) -> Fallback {
        if self.error.public_code == "auth_required" {
            Fallback::SignIn
        } else {
            Fallback::Blocked
        }
    }
}

/// What the gate already knows about a page that refused itda. `None` when
/// the answer was not a block.
pub fn refusal(url: &str, response: &itda_net::Response) -> Option<Refused> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned());
    if matches!(response.status, 301 | 302 | 303 | 307 | 308)
        && let Some((path, to)) = signin_redirect(url, response)
    {
        return Some(Refused {
            host,
            kind: Refusal::SignInRedirect { path, to },
            error: AppError::api("auth_required", 401),
        });
    }
    let error = itda_runtime::refusal(response, false);
    let kind = match (response.defense.as_str(), error.public_code.as_str()) {
        ("cloudflare", _) => Refusal::Cloudflare,
        ("bot-manager", _) => Refusal::BotManager,
        ("captcha", _) => Refusal::Captcha,
        (_, "site_rate_limited") => Refusal::RateLimited {
            wait: error
                .extra
                .get("retry_after")
                .and_then(serde_json::Value::as_u64),
        },
        (_, "blocked_by_site") => Refusal::Other {
            status: response.status,
        },
        _ => return None,
    };
    Some(Refused { host, kind, error })
}

/// The page's path and the sign-in page's address when the answer sends itda
/// on to a sign-in page (a `Location` with a path segment such as `login`,
/// `signin` or `sso`).
fn signin_redirect(url: &str, response: &itda_net::Response) -> Option<(String, String)> {
    let page = url::Url::parse(url).ok()?;
    let target = page.join(response.header("location")).ok()?;
    signin_path(&target).then(|| {
        (
            page.path().to_owned(),
            itda_types::url::sign_in_page(&target).to_string(),
        )
    })
}

/// Whether an address looks like a sign-in page: a path segment such as
/// `login`, `signin` or `sso`.
fn signin_path(url: &url::Url) -> bool {
    itda_types::url::looks_like_sign_in(url)
}

/// The sign-in a watched page ran into, from its recording: the page's own
/// request answered with a redirect to a sign-in page, or the last page the
/// browser showed on the site being a sign-in page its scripts went to.
/// `None` when the page stayed on itself.
pub fn signin_in_recording(url: &str, har: &serde_json::Value) -> Option<Refused> {
    let page = url::Url::parse(url).ok()?;
    if signin_path(&page) {
        return None;
    }
    let entries = har["log"]["entries"].as_array()?;
    let redirected = entries.iter().find_map(|entry| {
        let status = entry["response"]["status"].as_u64().unwrap_or(0);
        let request = entry["request"]["url"].as_str().unwrap_or_default();
        if !matches!(status, 301 | 302 | 303 | 307 | 308) || request != page.as_str() {
            return None;
        }
        entry["response"]["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|h| {
                h["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case("location"))
            })
            .filter_map(|h| page.join(h["value"].as_str()?).ok())
            .find(signin_path)
    });
    // The last page the browser showed on the site, not any page it
    // fetched: a public page may prefetch its own sign-in page.
    let opened = entries
        .iter()
        .filter(|entry| {
            entry["request"]["method"].as_str() == Some("GET")
                && entry["response"]["status"]
                    .as_u64()
                    .is_some_and(|s| (200..300).contains(&s))
                && entry["response"]["content"]["mimeType"]
                    .as_str()
                    .is_some_and(|m| m.contains("html"))
        })
        .filter_map(|entry| url::Url::parse(entry["request"]["url"].as_str()?).ok())
        .rfind(|u| u.host_str() == page.host_str())
        .filter(signin_path);
    let to = redirected.or(opened)?;
    Some(Refused {
        host: page.host_str().unwrap_or_default().to_owned(),
        kind: Refusal::SignInRedirect {
            path: page.path().to_owned(),
            to: itda_types::url::sign_in_page(&to).to_string(),
        },
        error: AppError::api("auth_required", 401),
    })
}

/// What the page offers, counted. The caller puts it in words.
#[derive(Debug, Default)]
pub struct PageSummary {
    /// How many records the list on the page holds, when there is one.
    pub records: Option<usize>,
    /// Data the page embeds, when there is no list.
    pub embedded_data: bool,
    /// The page is one thing with content of its own to read.
    pub main: bool,
    /// Forms other than a sign-in.
    pub forms: usize,
    pub feed: bool,
    pub description_link: bool,
    /// The page has a sign-in form as well: the site may need an account to
    /// be useful.
    pub sign_in_form: bool,
    /// The page was read with the session the person just signed in with.
    pub signed_in: bool,
    /// The frames followed on the page's own origin, by address, when the
    /// page was a shell; what they show is counted in with the page.
    pub frames: Vec<String>,
}

impl PageSummary {
    /// The summary of a page with the frame pages it showed: what any of
    /// them offers counts, and the frames are named.
    pub fn with_frames(mut self, framed: &frames::Framed) -> Self {
        for frame in &framed.pages {
            let inner = summarize(frame);
            self.main |= inner.main;
            self.records = match (self.records, inner.records) {
                (Some(a), Some(b)) => Some(a + b),
                (a, b) => a.or(b),
            };
            self.embedded_data |= inner.embedded_data;
            self.forms += inner.forms;
            self.feed |= inner.feed;
            self.description_link |= inner.description_link;
            self.sign_in_form |= inner.sign_in_form;
        }
        self.frames = framed.followed.clone();
        self
    }

    /// Nothing but a title: no records, no embedded data, no form, no feed
    /// and no API description. Such a page is worth watching in a browser
    /// when its scripts could fetch something.
    pub fn bare(&self) -> bool {
        self.records.is_none_or(|n| n == 0)
            && !self.embedded_data
            && self.forms == 0
            && !self.feed
            && !self.description_link
    }
}

/// The questions on the way, each answered by whoever runs the steps: a
/// person in a terminal, or fixed answers from an agent's tool call.
pub trait Answers {
    /// The page opens, but has a sign-in form: record in the browser instead
    /// of drafting from the page?
    fn record_anyway(&mut self) -> AppResult<bool>;
    /// The page needs a sign-in: the person's user name and password for
    /// the site's own sign-in form, which itda sends as itself. `None` when
    /// nobody is there to type them, or the person would rather record.
    fn sign_in(&mut self, host: &str) -> AppResult<Option<login::Credentials>>;
    /// The page could not be read on its own: open the browser and record?
    /// `false` stops without a definition.
    fn record(&mut self, why: Fallback) -> AppResult<bool>;
    /// Fires when the person is done recording; `None` when nobody can say
    /// so, and the recording ends when the window closes.
    fn recording_done(&mut self) -> Option<mpsc::Receiver<()>>;
    /// A site of that name is already set up: replace it?
    fn replace(&mut self, site: &str) -> AppResult<bool>;
    /// Whether anyone is there to be asked about keeping a credential.
    fn can_keep_credentials(&self) -> bool;
    /// Keep `what` (a header token, the browser session) as `site`'s default
    /// account in the keyring?
    fn keep_credential(&mut self, site: &str, what: &str) -> AppResult<bool>;
}

/// What happens, in order, as the steps run.
pub enum Progress<'a> {
    Looking {
        host: &'a str,
    },
    /// The site runs software itda knows; its template is used.
    Recognised(&'a software::Recognised),
    /// The page is on a site itda has a ready-made definition for; that
    /// definition is used and the page is not fetched.
    Bundled(&'a sites::Known),
    /// The page opened without a sign-in and this is what it offers.
    PageFetched(&'a PageSummary),
    /// A GraphQL endpoint was asked what it offers.
    SchemaAsked {
        url: &'a str,
    },
    /// The site's API description was read into the draft.
    Described(&'a Described),
    /// The page refused itda; what the gate saw.
    Refused(&'a Refused),
    /// The person's sign-in through the site's own form was accepted.
    SignedIn {
        host: &'a str,
        cookies: usize,
    },
    /// A browser window is opening on the page, to watch what it loads by
    /// itself; it closes on its own.
    Watching {
        host: &'a str,
    },
    /// The window closed: how many calls the page made, and how many reads
    /// beyond a title check the recording gave. Zero means the question
    /// comes next.
    Watched {
        calls: usize,
        reads: usize,
    },
    /// The page is not enough; recording is the way.
    NeedsRecord(Fallback),
    Recording(record::Progress),
    /// The draft, parsed: what agents will be able to do.
    Drafted(&'a Spec),
    /// Every read is about to be tried once.
    Trying(&'a Spec),
    /// One read was tried.
    Tried {
        spec: &'a Spec,
        outcome: &'a check::Outcome,
    },
    Saved {
        site: &'a str,
    },
    /// A credential the recording used was kept as `site/account`.
    CredentialKept {
        name: &'a str,
        site: &'a str,
        account: &'a str,
    },
}

/// A site set up: its definition as saved, with the reads that failed
/// already left out.
pub struct Done {
    pub site: String,
    pub path: PathBuf,
    pub spec: Spec,
    pub notes: Vec<String>,
    pub problems: Vec<String>,
    /// Every read tried, including the ones removed.
    pub outcomes: Vec<check::Outcome>,
    /// The actions removed because their read failed.
    pub removed: Vec<String>,
    /// Whether a credential the recording used, or the session the person
    /// signed in with, was kept.
    pub kept_credentials: bool,
    /// The software the site was recognised as, when a template was used.
    pub recognised: Option<software::Recognised>,
}

/// Why the steps ended without a site.
#[derive(Debug)]
pub enum Stopped {
    /// The page could not be read and recording was declined; `refused`
    /// says what the gate saw when the page refused itda.
    NoRecord {
        why: Fallback,
        refused: Option<Refused>,
    },
    /// A site of that name exists and replacing it was declined.
    Exists(String),
    /// No read worked, so nothing was kept.
    NothingWorks,
}

pub enum Finished {
    Done(Box<Done>),
    Stopped(Stopped),
}

/// Run every step. Questions go to `answers`; what happens goes to `report`.
pub async fn run(
    ctx: &Context,
    request: Request,
    answers: &mut (dyn Answers + Send),
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<Finished> {
    let url = itda_types::url::validate_url(&request.url)?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let forced = match &request.template {
        Template::Named(name) => Some(software::by_name(name).ok_or_else(|| {
            usage(
                format!("No template for software named {name}"),
                format!("Templates: {}.", software::names().join(", ")),
            )
        })?),
        _ => None,
    };
    let mut options = Options {
        name: request.name.clone(),
        base_url: None,
        include: None,
        lang: ctx.lang.clone(),
        schemas: Vec::new(),
        session: None,
        storage: None,
        frames_to_sign_in: Vec::new(),
    };

    // 1. Look at the page, or decide to record.
    report(Progress::Looking { host: &host });
    let mut record = request.record;
    let mut draft = None;
    let mut recognised = None;
    // The session the person signed in with, kept once the site is saved.
    let mut signed: Option<login::Signed> = None;
    // The recording a watched load or the person left: its pages are the
    // site's pages for a refresh.
    let mut har = Vec::new();
    // A site with a ready-made definition gets it, the way `itda add` would
    // install it, unless a template was asked for or turned off.
    let bundled = (!record && request.template == Template::Auto)
        .then(|| sites::known_host(&host))
        .flatten();
    if let Some(known) = &bundled {
        report(Progress::Bundled(known));
        draft = Some(bundled_draft(known)?);
    }
    if !record && bundled.is_none() {
        let probed = probe(ctx, url.as_str(), None).await?;
        // The software the site runs decides first, from the page's HTML,
        // confirmed by one request to its own API.
        let html = probed.page.as_ref().map(|page| page.html.as_str());
        recognised = if request.template == Template::Off {
            None
        } else if let Some(definition) = forced {
            match software::forced(definition, html, url.as_str()) {
                Some(hint) => Some(software::confirm(&ctx.net, &hint).await.unwrap_or(
                    software::Recognised {
                        software: definition.name,
                        title: hint.title,
                        base: hint.base,
                        version: None,
                        confirmed: false,
                    },
                )),
                None => None,
            }
        } else {
            match html.and_then(|html| software::fingerprint(html, url.as_str())) {
                Some(hint) => software::confirm(&ctx.net, &hint).await,
                None => None,
            }
        };
        if let Some(found) = &recognised {
            report(Progress::Recognised(found));
        }
        match probed.outcome {
            Ok(()) => {
                let page = probed.page.ok_or_else(|| AppError::api("internal", 500))?;
                // A shell whose content is in frames on its own origin is
                // read through them; the frame pages are pages of the site.
                let mut framed = frames::follow(&ctx.net, &page, None).await?;
                options.frames_to_sign_in = framed.sign_in_paths();
                let summary = summarize(&page).with_frames(&framed);
                report(Progress::PageFetched(&summary));
                if summary.sign_in_form && answers.record_anyway()? {
                    record = true;
                } else if let Some(Refused {
                    kind: Refusal::SignInRedirect { to, .. },
                    ..
                }) = &framed.sign_in
                {
                    // A frame of the shell answered with a redirect to a
                    // sign-in: treat it as the top-level page would be,
                    // signing in and reading the frame with the session.
                    let to = to.clone();
                    let need = SignInNeed {
                        why: Fallback::SignIn,
                        refused: framed.sign_in.take(),
                        sign_in: Some(to.as_str()),
                    };
                    match sign_in_or_record(
                        ctx,
                        &url,
                        need,
                        recognised.as_ref(),
                        &mut options,
                        answers,
                        report,
                    )
                    .await?
                    {
                        Drafted::Ready {
                            draft: d,
                            signed: s,
                        } => {
                            draft = Some(d);
                            signed = s;
                        }
                        Drafted::Record => record = true,
                        Drafted::Stopped(stopped) => return Ok(Finished::Stopped(stopped)),
                    }
                } else {
                    if recognised.is_none()
                        && request.watch
                        && summary.bare()
                        && evidence::fetches_by_itself(&page.html)
                    {
                        match watch_site(ctx, url.as_str(), &mut options, report).await? {
                            Watched::Drafted { draft: d, har: h } => {
                                draft = Some(d);
                                har = h;
                            }
                            Watched::SignIn(refused) => report(Progress::Refused(&refused)),
                            Watched::Nothing | Watched::NoBrowser => {}
                        }
                    }
                    if draft.is_none() {
                        let pages = with_frames(page, &mut framed);
                        draft = Some(
                            draft_pages(
                                ctx,
                                &pages,
                                &[],
                                &mut options,
                                recognised.as_ref(),
                                report,
                            )
                            .await?,
                        );
                    }
                }
            }
            Err((mut why, mut refused)) => {
                if recognised.is_none() && why == Fallback::Scripted && request.watch {
                    match watch_site(ctx, url.as_str(), &mut options, report).await? {
                        Watched::Drafted { draft: d, har: h } => {
                            draft = Some(d);
                            har = h;
                        }
                        Watched::SignIn(signin) => {
                            why = Fallback::SignIn;
                            refused = Some(signin);
                        }
                        Watched::Nothing | Watched::NoBrowser => {}
                    }
                }
                if draft.is_none() {
                    let need = SignInNeed {
                        why,
                        refused,
                        sign_in: probed.sign_in.as_deref(),
                    };
                    match sign_in_or_record(
                        ctx,
                        &url,
                        need,
                        recognised.as_ref(),
                        &mut options,
                        answers,
                        report,
                    )
                    .await?
                    {
                        Drafted::Ready {
                            draft: d,
                            signed: s,
                        } => {
                            draft = Some(d);
                            signed = s;
                        }
                        Drafted::Record => record = true,
                        Drafted::Stopped(stopped) => return Ok(Finished::Stopped(stopped)),
                    }
                }
            }
        }
    }
    let mut recording = None;
    if record {
        let rec = record_site(url.as_str(), answers.recording_done(), report).await?;
        har = serde_json::to_vec(&rec.har)?;
        let endpoints = itda_prep::graphql::endpoints_in_recording(&har);
        options.schemas = introspect(ctx, &endpoints, report).await;
        options.storage = record::storage_auth(&rec, &url);
        draft = Some(itda_prep::from_recording(&har, &options, "`itda setup`")?);
        recording = Some(rec);
    }
    let draft = draft.ok_or_else(|| AppError::api("internal", 500))?;
    let site = draft.site.clone();

    // 2. Show what was found.
    let spec = itda_spec::parse(draft.yaml.as_bytes())?;
    report(Progress::Drafted(&spec));
    // The recording stays in memory, so a refresh reads its pages again.
    let pages = if !har.is_empty() {
        itda_prep::pages_in_recording(&har, &spec.site.base_url)
    } else {
        vec![request.url.clone()]
    };
    let mut sources = refresh::Sources::of_pages(&pages, &draft, &options);
    if let Some(found) = &recognised {
        // A refresh fills the same template at the same API base.
        sources.template = Some(found.software.to_owned());
        sources.base_url = Some(found.base.clone());
    }

    // 3. Save, replacing only with a yes.
    let path = sites::specs_dir(&ctx.home).join(&site).join("spec.yaml");
    let replaced = path.exists();
    if replaced && !answers.replace(&site)? {
        return Ok(Finished::Stopped(Stopped::Exists(site)));
    }
    itda_types::util::atomic_file(&path, draft.yaml.as_bytes())?;
    if replaced {
        // What the user may have reviewed is no longer what would be sent.
        ctx.approvals.invalidate(Some(&site), None).await?;
    }
    let mut kept_credentials = false;
    if let Some(rec) = &recording {
        let spec = sites::load(&ctx.home, &site)?;
        kept_credentials = keep_credentials(ctx, &spec, rec, answers, report).await?;
    }
    if let Some(signed) = signed {
        // The person signed in for this site; without the session its
        // actions could not run, so it is kept as the default account, the
        // way the definition names it (cookies, or the query value).
        let account = itda_runtime::DEFAULT_ACCOUNT;
        let from = spec
            .find_auth("session")
            .map(|auth| auth.value_from.clone())
            .unwrap_or_default();
        login::keep(ctx, &site, account, "", signed.session, &from).await?;
        report(Progress::CredentialKept {
            name: itda_runtime::SESSION_SECRET,
            site: &site,
            account,
        });
        kept_credentials = true;
    }

    // 4. Try every read once; keep what works. A read the site refuses is
    // left out and nothing more is sent to it in this run.
    let spec = sites::load(&ctx.home, &site)?;
    report(Progress::Trying(&spec));
    let outcomes = check::try_once(&ctx.runtime, &spec).await;
    let mut removed = Vec::new();
    for outcome in &outcomes {
        report(Progress::Tried {
            spec: &spec,
            outcome,
        });
        if outcome.failed() {
            removed.push(outcome.action.clone());
        }
    }
    if check::nothing_worked(&spec, &outcomes) {
        let _ = std::fs::remove_file(&path);
        return Ok(Finished::Stopped(Stopped::NothingWorks));
    }
    if !removed.is_empty() {
        let yaml = without(&draft.yaml, &removed)?;
        itda_types::util::atomic_file(&path, yaml.as_bytes())?;
    }
    // A ready-made definition comes from itda itself, not from the site's
    // pages, so there is nothing to draft again from, nor sources left from
    // a site of the same name set up from its pages before.
    if bundled.is_none() {
        refresh::remember(&ctx.home, &site, sources)?;
    } else {
        let left = path.with_file_name(refresh::SOURCES_FILE);
        if left.exists() {
            std::fs::remove_file(left)?;
        }
    }
    report(Progress::Saved { site: &site });
    let spec = sites::load(&ctx.home, &site)?;
    Ok(Finished::Done(Box::new(Done {
        site,
        path,
        spec,
        notes: draft.notes,
        problems: draft.problems,
        outcomes,
        removed,
        kept_credentials,
        recognised,
    })))
}

/// A ready-made definition as a draft, so the same steps save and try it.
fn bundled_draft(known: &sites::Known) -> AppResult<itda_prep::Draft> {
    let yaml = sites::bundled(known.site).ok_or_else(|| AppError::api("internal", 500))?;
    let spec = itda_spec::parse(yaml.as_bytes())?;
    let reads = spec
        .operations
        .iter()
        .filter(|op| {
            itda_policy::operation_effect(op, &Default::default()) == itda_policy::Effect::Read
        })
        .count();
    Ok(itda_prep::Draft {
        site: spec.site.name.clone(),
        yaml: yaml.to_owned(),
        operations: spec.operations.len(),
        reads,
        thin_reads: 0,
        notes: Vec::new(),
        problems: Vec::new(),
        described: None,
    })
}

/// What fills a software's template for one site.
pub fn fill(found: &software::Recognised) -> itda_prep::Fill<'_> {
    itda_prep::Fill {
        template: found.definition().template,
        software: &found.title,
        base_url: &found.base,
    }
}

/// A page, fetched as itda.
pub struct Probed {
    /// The page when the answer was HTML, even when it is not enough to
    /// draft from: a sign-in page or an app shell still says what software
    /// made it.
    pub page: Option<Page>,
    /// `Err` says why the page is not enough, with what the gate saw when
    /// the page refused itda.
    pub outcome: Result<(), (Fallback, Option<Refused>)>,
    /// The sign-in page the answer pointed at, when the page needs one: the
    /// redirect's target, the page's own sign-in form, or the sign-in link
    /// on a "log in to see this" page.
    pub sign_in: Option<String>,
}

impl Probed {
    fn not_enough(why: Fallback, refused: Option<Refused>, sign_in: Option<String>) -> Self {
        Self {
            page: None,
            outcome: Err((why, refused)),
            sign_in,
        }
    }
}

/// The page with the frame pages it showed, as the pages to draft from.
fn with_frames(page: Page, framed: &mut frames::Framed) -> Vec<Page> {
    let mut pages = vec![page];
    pages.append(&mut framed.pages);
    pages
}

/// Fetch the page as itda, with the `session` when the person has signed
/// in: its cookies in the header, and a session the site carries in the
/// query on the address, which the page's own address then does without.
pub async fn probe(
    ctx: &Context,
    url: &str,
    session: Option<&login::Session>,
) -> AppResult<Probed> {
    let session_key = session
        .and_then(|s| s.query.as_ref())
        .map(|(key, _)| key.as_str());
    let address = match session.and_then(|s| s.query.as_ref()) {
        Some((key, value)) => itda_types::url::with_query_pair(url, key, value),
        None => url.to_owned(),
    };
    let mut request = itda_net::Request {
        limit: 8 * 1024 * 1024,
        ..itda_net::Request::read(address)
    };
    let cookies = session
        .map(|s| s.cookies.as_str())
        .filter(|c| !c.is_empty());
    if let Some(cookies) = cookies {
        request.headers.push(("cookie".into(), cookies.to_owned()));
    }
    let response = match ctx.net.fetch(request).await {
        Ok(response) => response,
        Err(error)
            if matches!(
                error.public_code.as_str(),
                "blocked_by_site" | "site_rate_limited"
            ) =>
        {
            return Ok(Probed::not_enough(Fallback::Blocked, None, None));
        }
        Err(error) => return Err(error),
    };
    if let Some(refused) = refusal(url, &response) {
        let sign_in = match &refused.kind {
            Refusal::SignInRedirect { to, .. } => Some(to.clone()),
            _ => None,
        };
        return Ok(Probed::not_enough(
            refused.fallback(),
            Some(refused),
            sign_in,
        ));
    }
    if matches!(response.status, 401 | 403) {
        let sign_in = response
            .header("content-type")
            .contains("html")
            .then(|| {
                evidence::sign_in_link(&String::from_utf8_lossy(&response.bytes), &response.url)
            })
            .flatten();
        return Ok(Probed::not_enough(Fallback::SignIn, None, sign_in));
    }
    if !response.successful() {
        return Ok(Probed::not_enough(Fallback::Blocked, None, None));
    }
    if !response.header("content-type").contains("html") {
        return Err(usage(
            format!("{url} is not an HTML page"),
            "Pass the address of a page. For an API description, use `itda draft openapi`.",
        ));
    }
    let html = String::from_utf8_lossy(&response.bytes).into_owned();
    let no_records = evidence::infer_extraction(&html, &response.url).is_none();
    let signed_in_form = evidence::forms(&html, &response.url)
        .iter()
        .any(|form| form.password)
        && no_records;
    let page = Page {
        url: match session_key {
            Some(key) => itda_types::url::without_query_key(&response.url, key),
            None => response.url.clone(),
        },
        html,
        link: response.headers_joined("link"),
    };
    let mut sign_in = None;
    // A shell with frames on its own origin is read through them before it
    // counts as drawn by scripts.
    let outcome =
        if evidence::script_shell(&page.html) && !frames::has_own_frames(&page.html, &page.url) {
            Err((Fallback::Scripted, None))
        } else if signed_in_form && response.url.to_lowercase().contains("login") {
            sign_in = url::Url::parse(&response.url)
                .map(|url| itda_types::url::sign_in_page(&url).to_string())
                .ok()
                .or_else(|| Some(response.url.clone()));
            Err((Fallback::SignIn, None))
        } else if no_records
            && !signed_in_form
            && session.is_none()
            && login_required_title(&page.html)
            && let Some(link) = evidence::sign_in_link(&page.html, &response.url)
        {
            // A page that says "log in to see this" instead of redirecting.
            sign_in = Some(link);
            Err((Fallback::SignIn, None))
        } else {
            Ok(())
        };
    Ok(Probed {
        page: Some(page),
        outcome,
        sign_in,
    })
}

/// Whether the page's title says a sign-in is needed ("Login required",
/// "Please sign in", "로그인이 필요합니다").
fn login_required_title(html: &str) -> bool {
    let title = itda_html::title(html).to_lowercase();
    ["login", "log in", "sign in", "sign-in", "로그인"]
        .iter()
        .any(|word| title.contains(word))
}

/// Sign in through the site's own form at `sign_in` as itda, then fetch the
/// page the person asked for with the session. The session comes back with
/// the page, to keep once the definition is saved.
async fn sign_in_as_itda(
    ctx: &Context,
    url: &url::Url,
    sign_in: &str,
    credentials: &login::Credentials,
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<(Page, login::Signed)> {
    // Where the session will be kept is checked before the password is sent.
    credentials::writable()?;
    let signed = login::form(&ctx.net, url, sign_in, None, credentials, None).await?;
    report(Progress::SignedIn {
        host: url.host_str().unwrap_or_default(),
        cookies: signed.session.count,
    });
    let probed = probe(ctx, url.as_str(), Some(&signed.session)).await?;
    let host = url.host_str().unwrap_or_default();
    match probed.outcome {
        Ok(()) => {
            let page = probed.page.ok_or_else(|| AppError::api("internal", 500))?;
            Ok((page, signed))
        }
        Err((Fallback::Scripted, _)) => Err(usage(
            format!("{host} accepted the sign-in, but scripts draw the page"),
            "Record yourself using the site instead: itda record <url>.",
        )),
        Err(_) => Err(usage(
            format!("{host} accepted the sign-in, but still does not show the page"),
            "Check the address in a browser, signed in, and try again.",
        )),
    }
}

/// A page that is not enough on its own needs a sign-in, or could not be read
/// for another reason: what the gate saw, and the sign-in page to use. The
/// sign-in may have been found on the top-level page or in one of its frames.
struct SignInNeed<'a> {
    why: Fallback,
    refused: Option<Refused>,
    sign_in: Option<&'a str>,
}

/// What the sign-in-or-record tail decided: a draft, with the kept session
/// when a sign-in gave one; that the person chose to record; or that they
/// declined and setup stops.
enum Drafted {
    Ready {
        draft: Draft,
        signed: Option<login::Signed>,
    },
    Record,
    Stopped(Stopped),
}

/// The one path a page that is not enough on its own takes: sign in through
/// the site's own form and draft the page as the person sees it, fall back to
/// the software's template when the software is known, or record. Shared by
/// a sign-in found on the top-level page and one found in a frame.
async fn sign_in_or_record(
    ctx: &Context,
    url: &url::Url,
    need: SignInNeed<'_>,
    recognised: Option<&software::Recognised>,
    options: &mut Options,
    answers: &mut (dyn Answers + Send),
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<Drafted> {
    let host = url.host_str().unwrap_or_default().to_owned();
    if let Some(refused) = &need.refused {
        report(Progress::Refused(refused));
    }
    // The page needs a sign-in and the person is there to type it: itda signs
    // in through the site's own form and reads the page as they see it.
    let credentials = match (need.why, need.sign_in) {
        (Fallback::SignIn, Some(_)) => answers.sign_in(&host)?,
        _ => None,
    };
    if let (Some(credentials), Some(to)) = (credentials, need.sign_in) {
        let (page, session) = sign_in_as_itda(ctx, url, to, &credentials, report).await?;
        let mut framed = frames::follow(&ctx.net, &page, Some(&session.session)).await?;
        let mut summary = summarize(&page).with_frames(&framed);
        summary.signed_in = true;
        report(Progress::PageFetched(&summary));
        options.session = Some(itda_prep::SessionAuth {
            cookie: session.cookie.clone().unwrap_or_default(),
            query: session.session.query.as_ref().map(|(key, _)| key.clone()),
            login: session.form.clone(),
        });
        let pages = with_frames(page, &mut framed);
        let draft = draft_pages(ctx, &pages, &[], options, recognised, report).await?;
        Ok(Drafted::Ready {
            draft,
            signed: Some(session),
        })
    } else if let Some(found) = recognised {
        // A page that is not enough on its own still gets the software's API
        // when the software is known.
        let draft = itda_prep::from_template(&fill(found), &[], None, options)?;
        Ok(Drafted::Ready {
            draft,
            signed: None,
        })
    } else {
        report(Progress::NeedsRecord(need.why));
        if !answers.record(need.why)? {
            return Ok(Drafted::Stopped(Stopped::NoRecord {
                why: need.why,
                refused: need.refused,
            }));
        }
        Ok(Drafted::Record)
    }
}

/// What the page offers, counted.
pub fn summarize(page: &Page) -> PageSummary {
    let mut summary = PageSummary::default();
    let extraction = evidence::infer_extraction(&page.html, &page.url);
    // The same rule the draft applies: a page that embeds lists is a list.
    summary.main = !itda_prep::embeds_lists(&page.html)
        && evidence::main_content(&page.html, extraction.as_ref().map(|e| e.items.as_str()))
            .is_some();
    if let Some(extraction) = extraction
        && let Some(value) = extraction.extract(&page.html, &page.url)
    {
        summary.records = Some(value["items"].as_array().map(Vec::len).unwrap_or(0));
    } else if !summary.main && !itda_html::extract_ssr_map(&page.html).is_empty() {
        summary.embedded_data = true;
    }
    let forms = evidence::forms(&page.html, &page.url);
    summary.forms = forms.iter().filter(|f| !f.password).count();
    summary.sign_in_form = forms.iter().any(|f| f.password);
    summary.feed = !evidence::feeds(&page.html, &page.url).is_empty();
    summary.description_link = evidence::openapi_link(&page.html, &page.url).is_some();
    summary
}

/// Draft from one page: ask the GraphQL endpoints it names, read the API
/// description it publishes, then draft from all of it; with `software`
/// recognised, its template comes first and the page's reads join it.
pub async fn draft_page(
    ctx: &Context,
    page: &Page,
    options: &mut Options,
    software: Option<&software::Recognised>,
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<Draft> {
    draft_pages(
        ctx,
        std::slice::from_ref(page),
        &[],
        options,
        software,
        report,
    )
    .await
}

/// Draft from pages: ask the GraphQL endpoints they name (and `endpoints`,
/// known from before), read the API description they publish, then draft
/// from all of it.
pub async fn draft_pages(
    ctx: &Context,
    pages: &[Page],
    endpoints: &[String],
    options: &mut Options,
    software: Option<&software::Recognised>,
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<Draft> {
    let mut endpoints: Vec<String> = endpoints.to_vec();
    for endpoint in introspect::endpoints_in_pages(pages) {
        if !endpoints.contains(&endpoint) {
            endpoints.push(endpoint);
        }
    }
    options.schemas = introspect(ctx, &endpoints, report).await;
    let description = description::find(&ctx.net, pages).await;
    let draft = match software {
        Some(found) => {
            itda_prep::from_template(&fill(found), pages, description.as_ref(), options)?
        }
        None => itda_prep::from_pages_and_description(pages, description.as_ref(), options)?,
    };
    if let Some(described) = &draft.described {
        report(Progress::Described(described));
    }
    Ok(draft)
}

async fn introspect(
    ctx: &Context,
    endpoints: &[String],
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> Vec<itda_prep::graphql::Schema> {
    if endpoints.is_empty() {
        return Vec::new();
    }
    let schemas = introspect::schemas(&ctx.net, endpoints).await;
    for schema in &schemas {
        report(Progress::SchemaAsked { url: &schema.url });
    }
    schemas
}

/// What watching a page load in the browser gave.
enum Watched {
    /// Reads drafted from the calls the page made by itself, with the
    /// recording they came from.
    Drafted { draft: Draft, har: Vec<u8> },
    /// The page made no call worth a read; the question comes next.
    Nothing,
    /// The browser went to a sign-in page instead; nobody typed anything.
    SignIn(Refused),
    /// No browser on this computer; the step is skipped without a word.
    NoBrowser,
}

/// Open the browser on the page, watch what it loads by itself and on one
/// scroll, and draft from that as `itda draft har` would. The window is the
/// same fresh profile recording uses; nothing in it is clicked or typed, and
/// what it signed in with is never kept, since nobody signed in.
async fn watch_site(
    ctx: &Context,
    url: &str,
    options: &mut Options,
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<Watched> {
    let Ok(browser) = itda_browser::find() else {
        return Ok(Watched::NoBrowser);
    };
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    report(Progress::Watching { host: &host });
    let page = url.to_owned();
    let rec = tokio::task::spawn_blocking(move || record::watch(&browser, &page))
        .await
        .map_err(|_| AppError::api("internal", 500))??;
    let calls = rec.har["log"]["entries"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    if let Some(refused) = signin_in_recording(url, &rec.har) {
        report(Progress::Watched { calls, reads: 0 });
        return Ok(Watched::SignIn(refused));
    }
    let har = serde_json::to_vec(&rec.har)?;
    let endpoints = itda_prep::graphql::endpoints_in_recording(&har);
    options.schemas = introspect(ctx, &endpoints, report).await;
    // The page's own address first, so what the page itself shows is never
    // lost to a busier API elsewhere; then the way `itda draft har` picks.
    let origin = url::Url::parse(url)
        .map(|u| u.origin().ascii_serialization())
        .ok();
    let mut best: Option<Draft> = None;
    for base in [origin, None] {
        options.base_url = base;
        let draft = match itda_prep::from_recording(&har, options, "`itda setup`") {
            Ok(draft) => draft,
            Err(error) if itda_prep::drafted_nothing(&error) => continue,
            Err(error) => return Err(error),
        };
        if draft.reads > draft.thin_reads {
            best = Some(draft);
            break;
        }
    }
    let Some(draft) = best else {
        options.base_url = None;
        options.schemas.clear();
        report(Progress::Watched { calls, reads: 0 });
        return Ok(Watched::Nothing);
    };
    report(Progress::Watched {
        calls,
        reads: draft.reads - draft.thin_reads,
    });
    Ok(Watched::Drafted { draft, har })
}

/// Record off the async runtime, passing what the recording says along as
/// it happens.
async fn record_site(
    url: &str,
    done: Option<mpsc::Receiver<()>>,
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<record::Recording> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let url = url.to_owned();
    let mut task = tokio::task::spawn_blocking(move || {
        record::browser(&url, done.as_ref(), &mut |progress| {
            let _ = sender.send(progress);
        })
    });
    let outcome = loop {
        tokio::select! {
            Some(progress) = receiver.recv() => report(Progress::Recording(progress)),
            finished = &mut task => break finished,
        }
    };
    while let Ok(progress) = receiver.try_recv() {
        report(Progress::Recording(progress));
    }
    outcome.map_err(|_| AppError::api("internal", 500))?
}

/// Offer what the recording signed in with, one yes at a time. Nothing is
/// kept where nobody can answer, or where secrets are read-only.
async fn keep_credentials(
    ctx: &Context,
    spec: &Spec,
    rec: &record::Recording,
    answers: &mut (dyn Answers + Send),
    report: &mut (dyn FnMut(Progress<'_>) + Send),
) -> AppResult<bool> {
    if !answers.can_keep_credentials() || credentials::store_is_env() {
        return Ok(false);
    }
    let offers = credentials::offers(spec, &rec.har, &rec.cookies, &rec.storage);
    if offers.is_empty() {
        return Ok(false);
    }
    itda_vault::KeyringStore::available()?;
    let site = &spec.site.name;
    let account = itda_runtime::DEFAULT_ACCOUNT;
    let mut kept = false;
    for offer in offers {
        if !answers.keep_credential(site, &offer.what)? {
            continue;
        }
        credentials::keep(ctx, site, account, &offer.name, offer.secret).await?;
        report(Progress::CredentialKept {
            name: &offer.name,
            site,
            account,
        });
        kept = true;
    }
    Ok(kept)
}

/// Lift pauses on every origin the site's actions use.
pub async fn resume(ctx: &Context, site: &str, account: Option<&str>) -> AppResult<()> {
    let spec = sites::load(&ctx.home, site)?;
    for origin in browser_origins(&spec) {
        ctx.net.resume(&origin, account.unwrap_or_default()).await?;
    }
    Ok(())
}

pub fn browser_origins(spec: &Spec) -> Vec<String> {
    let mut origins: Vec<String> = spec
        .operations
        .iter()
        .map(|op| spec.operation_base(op))
        .chain(std::iter::once(spec.site.base_url.as_str()))
        .filter_map(|base| url::Url::parse(base).ok())
        .map(|url| url.origin().ascii_serialization())
        .collect();
    origins.sort();
    origins.dedup();
    origins
}

pub async fn reset_browser_sessions(ctx: &Context, spec: &Spec) -> AppResult<()> {
    let mut accounts = ctx
        .vault
        .accounts(Some(&spec.site.name))
        .await?
        .into_iter()
        .map(|account| account.id)
        .collect::<Vec<_>>();
    accounts.extend([String::new(), itda_runtime::DEFAULT_ACCOUNT.to_owned()]);
    accounts.sort();
    accounts.dedup();
    for origin in browser_origins(spec) {
        for account in &accounts {
            ctx.net.reset_browser(&origin, account).await?;
        }
    }
    Ok(())
}

/// The definition without the named operations, with its comments kept.
pub fn without(yaml: &str, names: &[String]) -> AppResult<String> {
    edit_operations(yaml, |ops| {
        ops.retain(|op| {
            op.get("name")
                .and_then(serde_norway::Value::as_str)
                .is_none_or(|name| !names.iter().any(|n| n == name))
        });
    })
}

fn edit_operations(
    yaml: &str,
    edit: impl FnOnce(&mut serde_norway::Sequence),
) -> AppResult<String> {
    let (header, body): (Vec<&str>, Vec<&str>) = yaml.lines().partition(|l| l.starts_with('#'));
    let mut doc: serde_norway::Value =
        serde_norway::from_str(&body.join("\n")).map_err(|_| AppError::api("internal", 500))?;
    if let Some(serde_norway::Value::Sequence(ops)) = doc.get_mut("operations") {
        edit(ops);
    }
    let rest = serde_norway::to_string(&doc).map_err(|_| AppError::api("internal", 500))?;
    Ok(format!("{}\n{rest}", header.join("\n")))
}

/// The data directory's `specs/<site>/spec.yaml`, where `itda setup` saves.
pub fn spec_path(home: &Path, site: &str) -> PathBuf {
    sites::specs_dir(home).join(site).join("spec.yaml")
}

#[cfg(test)]
mod tests {
    use super::without;

    const YAML: &str = "# drafted by a test
spec_version: 1
site: {name: board, base_url: 'https://board.example'}
operations:
  - {name: get-board, desc: 'List what Board shows', method: GET, path: /board, effect: read, evidence: 'the page was fetched'}
  - {name: feed, method: GET, path: /feed.xml, effect: read, evidence: doc}
";

    #[test]
    fn removing_an_operation_keeps_the_header_comment() {
        let yaml = without(YAML, &["feed".into()]).unwrap();
        assert!(yaml.starts_with("# drafted by a test\n"), "{yaml}");
        assert!(yaml.contains("get-board"));
        assert!(!yaml.contains("feed.xml"));
    }
}
