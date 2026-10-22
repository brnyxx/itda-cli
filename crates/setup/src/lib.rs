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
    /// The answer sends itda to a sign-in page instead of showing `path`.
    SignInRedirect {
        path: String,
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
        && let Some(path) = signin_redirect(url, response)
    {
        return Some(Refused {
            host,
            kind: Refusal::SignInRedirect { path },
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

/// The page's path when its answer sends itda on to a sign-in page (a
/// `Location` with a path segment such as `login`, `signin` or `sso`).
fn signin_redirect(url: &str, response: &itda_net::Response) -> Option<String> {
    let page = url::Url::parse(url).ok()?;
    let target = page.join(response.header("location")).ok()?;
    let signin = target.path().split('/').any(|segment| {
        matches!(
            segment.to_ascii_lowercase().as_str(),
            "login" | "signin" | "sign-in" | "sign_in" | "sso" | "oauth" | "authenticate"
        )
    });
    signin.then(|| page.path().to_owned())
}

/// What the page offers, counted. The caller puts it in words.
#[derive(Debug, Default)]
pub struct PageSummary {
    /// How many records the list on the page holds, when there is one.
    pub records: Option<usize>,
    /// Data the page embeds, when there is no list.
    pub embedded_data: bool,
    /// Forms other than a sign-in.
    pub forms: usize,
    pub feed: bool,
    pub description_link: bool,
    /// The page has a sign-in form as well: the site may need an account to
    /// be useful.
    pub sign_in_form: bool,
}

/// The questions on the way, each answered by whoever runs the steps: a
/// person in a terminal, or fixed answers from an agent's tool call.
pub trait Answers {
    /// The page opens, but has a sign-in form: record in the browser instead
    /// of drafting from the page?
    fn record_anyway(&mut self) -> AppResult<bool>;
    /// The page could not be read as itself: open the browser and record?
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
    /// Whether a credential the recording used was kept.
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
    };

    // 1. Look at the page, or decide to record.
    report(Progress::Looking { host: &host });
    let mut record = request.record;
    let mut draft = None;
    let mut recognised = None;
    if !record {
        let probed = probe(ctx, url.as_str()).await?;
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
                let summary = summarize(&page);
                report(Progress::PageFetched(&summary));
                if summary.sign_in_form && answers.record_anyway()? {
                    record = true;
                } else {
                    draft = Some(
                        draft_page(ctx, &page, &mut options, recognised.as_ref(), report).await?,
                    );
                }
            }
            Err((why, refused)) => {
                if let Some(refused) = &refused {
                    report(Progress::Refused(refused));
                }
                // A page that is not enough on its own still gets the
                // software's API when the software is known.
                if let Some(found) = &recognised {
                    draft = Some(itda_prep::from_template(&fill(found), &[], None, &options)?);
                } else {
                    report(Progress::NeedsRecord(why));
                    if !answers.record(why)? {
                        return Ok(Finished::Stopped(Stopped::NoRecord { why, refused }));
                    }
                    record = true;
                }
            }
        }
    }
    let mut recording = None;
    let mut har = Vec::new();
    if record {
        let rec = record_site(url.as_str(), answers.recording_done(), report).await?;
        har = serde_json::to_vec(&rec.har)?;
        let endpoints = itda_prep::graphql::endpoints_in_recording(&har);
        options.schemas = introspect(ctx, &endpoints, report).await;
        draft = Some(itda_prep::from_recording(&har, &options, "`itda setup`")?);
        recording = Some(rec);
    }
    let draft = draft.ok_or_else(|| AppError::api("internal", 500))?;
    let site = draft.site.clone();

    // 2. Show what was found.
    let spec = itda_spec::parse(draft.yaml.as_bytes())?;
    report(Progress::Drafted(&spec));
    // The recording stays in memory, so a refresh reads its pages again.
    let pages = if record {
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
    if path.exists() && !answers.replace(&site)? {
        return Ok(Finished::Stopped(Stopped::Exists(site)));
    }
    itda_types::util::atomic_file(&path, draft.yaml.as_bytes())?;
    let mut kept_credentials = false;
    if let Some(rec) = &recording {
        let spec = sites::load(&ctx.home, &site)?;
        kept_credentials = keep_credentials(ctx, &spec, rec, answers, report).await?;
    }

    // 4. Try every read; keep what works.
    let spec = sites::load(&ctx.home, &site)?;
    report(Progress::Trying(&spec));
    let outcomes = check::run(&ctx.runtime, std::slice::from_ref(&spec), None).await;
    let mut removed = Vec::new();
    let mut worked = 0;
    for outcome in &outcomes {
        report(Progress::Tried {
            spec: &spec,
            outcome,
        });
        match outcome.status.as_str() {
            "ok" => worked += 1,
            "skipped" => {}
            _ => removed.push(outcome.action.clone()),
        }
    }
    let kept = spec.operations.len() - removed.len();
    let all_reads = spec.operations.iter().all(|op| {
        itda_policy::operation_effect(op, &Default::default()) == itda_policy::Effect::Read
    });
    if kept == 0 || (worked == 0 && all_reads) {
        let _ = std::fs::remove_file(&path);
        return Ok(Finished::Stopped(Stopped::NothingWorks));
    }
    if !removed.is_empty() {
        let yaml = without(&draft.yaml, &removed)?;
        itda_types::util::atomic_file(&path, yaml.as_bytes())?;
    }
    refresh::remember(&ctx.home, &site, sources)?;
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
}

impl Probed {
    fn not_enough(why: Fallback, refused: Option<Refused>) -> Self {
        Self {
            page: None,
            outcome: Err((why, refused)),
        }
    }
}

/// Fetch the page as itda.
pub async fn probe(ctx: &Context, url: &str) -> AppResult<Probed> {
    let request = itda_net::Request {
        limit: 8 * 1024 * 1024,
        ..itda_net::Request::read(url)
    };
    let response = match ctx.net.fetch(request).await {
        Ok(response) => response,
        Err(error)
            if matches!(
                error.public_code.as_str(),
                "blocked_by_site" | "site_rate_limited"
            ) =>
        {
            return Ok(Probed::not_enough(Fallback::Blocked, None));
        }
        Err(error) => return Err(error),
    };
    if let Some(refused) = refusal(url, &response) {
        return Ok(Probed::not_enough(refused.fallback(), Some(refused)));
    }
    if matches!(response.status, 401 | 403) {
        return Ok(Probed::not_enough(Fallback::SignIn, None));
    }
    if !response.successful() {
        return Ok(Probed::not_enough(Fallback::Blocked, None));
    }
    if !response.header("content-type").contains("html") {
        return Err(usage(
            format!("{url} is not an HTML page"),
            "Pass the address of a page. For an API description, use `itda draft openapi`.",
        ));
    }
    let html = String::from_utf8_lossy(&response.bytes).into_owned();
    let signed_in_form = evidence::forms(&html, &response.url)
        .iter()
        .any(|form| form.password)
        && evidence::infer_extraction(&html, &response.url).is_none();
    let page = Page {
        url: response.url.clone(),
        html,
    };
    let outcome = if evidence::script_shell(&page.html) {
        Err((Fallback::Scripted, None))
    } else if signed_in_form && response.url.to_lowercase().contains("login") {
        Err((Fallback::SignIn, None))
    } else {
        Ok(())
    };
    Ok(Probed {
        page: Some(page),
        outcome,
    })
}

/// What the page offers, counted.
pub fn summarize(page: &Page) -> PageSummary {
    let mut summary = PageSummary::default();
    if let Some(extraction) = evidence::infer_extraction(&page.html, &page.url)
        && let Some(value) = extraction.extract(&page.html, &page.url)
    {
        summary.records = Some(value["items"].as_array().map(Vec::len).unwrap_or(0));
    } else if !itda_html::extract_ssr_map(&page.html).is_empty() {
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
    let offers = credentials::offers(spec, &rec.har, &rec.cookies);
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
    let mut origins: Vec<String> = spec
        .operations
        .iter()
        .filter_map(|op| url::Url::parse(spec.operation_base(op)).ok())
        .map(|url| url.origin().ascii_serialization())
        .collect();
    origins.sort();
    origins.dedup();
    for origin in &origins {
        ctx.net.resume(origin, account.unwrap_or_default()).await?;
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
