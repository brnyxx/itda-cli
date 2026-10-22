//! `itda setup <url>`: one command from a page address to an agent that can
//! use the site. Each step is one of the existing commands; this only asks
//! the questions and keeps what works.
use crate::check;
use crate::text::{client_label, t, tf};
use itda_html::evidence;
use itda_policy::{Effect, operation_effect};
use itda_spec::Spec;
use itda_types::error::{AppError, AppResult, usage};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal, Write},
    path::Path,
    process::ExitCode,
};

pub struct Flags {
    pub url: String,
    pub name: Option<String>,
    /// `Some(true)` records, `Some(false)` never opens the browser, `None` asks.
    pub record: Option<bool>,
    pub client: Option<String>,
    pub yes: bool,
}

/// Why a fetched page is not enough to draft from.
enum Fallback {
    Blocked,
    SignIn,
    Scripted,
}

pub async fn run(home: &Path, flags: Flags) -> AppResult<ExitCode> {
    let interactive = std::io::stdin().is_terminal();
    if !interactive && (flags.record.is_none() || flags.client.is_none()) {
        return Err(usage(t("no_terminal"), ""));
    }
    let url = itda_types::url::validate_url(&flags.url)?;
    let host = url.host_str().unwrap_or_default().to_owned();
    let mut options = itda_prep::Options {
        name: flags.name.clone(),
        base_url: None,
        include: None,
        lang: crate::lang(),
        schemas: Vec::new(),
    };

    // 1. Look at the page, or decide to record.
    step(1, &tf("looking", &[("host", &host)]));
    let mut record = flags.record == Some(true);
    let mut draft = None;
    if !record {
        match probe(home, url.as_str()).await? {
            Ok(page) => {
                eprintln!("    {}", t("fetched_page"));
                eprintln!("    {}", tf("found", &[("what", &describe_page(&page))]));
                let has_sign_in = evidence::forms(&page.html, &page.url)
                    .iter()
                    .any(|form| form.password);
                if flags.record.is_none() && has_sign_in && ask(t("ask_record_anyway"), false)? {
                    record = true;
                } else {
                    let pages = [page];
                    options.schemas =
                        introspect(home, &crate::introspect::endpoints_in_pages(&pages)).await?;
                    let net = crate::open(home)?.net;
                    let description = crate::description::find(&net, &pages).await;
                    let drafted = itda_prep::from_pages_and_description(
                        &pages,
                        description.as_ref(),
                        &options,
                    )?;
                    if let Some(described) = &drafted.described {
                        eprintln!("    {}", crate::text::described(described));
                    }
                    draft = Some(drafted);
                }
            }
            Err(why) => {
                let why = match why {
                    Fallback::Blocked => t("why_blocked"),
                    Fallback::SignIn => t("why_signin"),
                    Fallback::Scripted => t("why_scripted"),
                };
                eprintln!("    {}", tf("needs_record", &[("why", why)]));
                if flags.record == Some(false) {
                    return Err(usage(t("no_record_allowed"), ""));
                }
                if !ask(t("ask_record"), true)? {
                    return Ok(ExitCode::FAILURE);
                }
                record = true;
            }
        }
    }
    let mut recording = None;
    if record {
        let start = url.to_string();
        let rec = tokio::task::spawn_blocking(move || crate::record::browser(&start))
            .await
            .map_err(|_| AppError::api("internal", 500))??;
        let har = serde_json::to_vec(&rec.har)?;
        options.schemas =
            introspect(home, &itda_prep::graphql::endpoints_in_recording(&har)).await?;
        draft = Some(itda_prep::from_recording(&har, &options, "`itda setup`")?);
        recording = Some(rec);
    }
    let draft = draft.ok_or_else(|| AppError::api("internal", 500))?;
    let site = draft.site.clone();

    // 2. Show what was found, in words.
    step(2, t("actions_heading"));
    let spec = itda_spec::parse(draft.yaml.as_bytes())?;
    list_actions(&spec);

    // 3. Save, replacing only with a yes.
    let path = crate::sites::specs_dir(home).join(&site).join("spec.yaml");
    if path.exists() && !flags.yes && !ask(&tf("exists", &[("site", &site)]), false)? {
        eprintln!("    {}", tf("exists_kept", &[("site", &site)]));
        return Ok(ExitCode::FAILURE);
    }
    itda_types::util::atomic_file(&path, draft.yaml.as_bytes())?;
    if let Some(rec) = &recording {
        let spec = crate::sites::load(home, &site)?;
        crate::keep_credentials(home, &spec, &rec.har, &rec.cookies).await?;
    }

    // 4. Try every read; keep what works.
    step(3, t("trying"));
    let spec = crate::sites::load(home, &site)?;
    let outcomes = check::run(
        &crate::open(home)?.runtime(),
        std::slice::from_ref(&spec),
        None,
    )
    .await;
    let mut failed = Vec::new();
    let mut worked = 0;
    let width = name_width(&spec);
    for o in &outcomes {
        let op = spec.op(&o.action)?;
        let line = match o.status.as_str() {
            "ok" => {
                worked += 1;
                match o.records {
                    Some(n) => format!(
                        "✓ {:<width$} {}",
                        o.action,
                        tf("ok_records", &[("n", &n.to_string())])
                    ),
                    None => format!("✓ {:<width$} {}", o.action, t("ok")),
                }
            }
            "skipped" if operation_effect(op, &BTreeMap::new()) != Effect::Read => {
                format!("· {:<width$} {}", o.action, t("skipped_change"))
            }
            "skipped" => format!(
                "· {:<width$} {}",
                o.action,
                tf(
                    "skipped_needs",
                    &[("list", &check::needed_inputs(op).join(", "))]
                )
            ),
            code => {
                failed.push(o.action.clone());
                let why = match (code, o.retry_after_s) {
                    ("response_mismatch", _) => t("why_mismatch").to_owned(),
                    ("auth_required", _) => t("why_auth").to_owned(),
                    ("not_found", _) => t("why_not_found").to_owned(),
                    ("blocked_by_site", _) => t("why_blocked").to_owned(),
                    ("site_rate_limited", Some(seconds)) => {
                        tf("why_wait", &[("n", &seconds.to_string())])
                    }
                    ("site_rate_limited", None) => t("why_rate_limited").to_owned(),
                    _ => o.detail.clone(),
                };
                format!("✗ {:<width$} {}", o.action, tf("removed", &[("why", &why)]))
            }
        };
        eprintln!("    {line}");
    }
    let kept = spec.operations.len() - failed.len();
    if kept == 0
        || (worked == 0
            && spec
                .operations
                .iter()
                .all(|op| operation_effect(op, &BTreeMap::new()) == Effect::Read))
    {
        let _ = std::fs::remove_file(&path);
        return Err(usage(tf("nothing_works", &[("url", url.as_str())]), ""));
    }
    if !failed.is_empty() {
        let yaml = without(&draft.yaml, &failed)?;
        itda_types::util::atomic_file(&path, yaml.as_bytes())?;
    }
    step(4, &tf("saved", &[("site", &site)]));

    // 5. Connect an agent.
    let clients = match &flags.client {
        Some(client) if client == "none" => Vec::new(),
        Some(client) => vec![client.clone()],
        None => choose_clients()?,
    };
    if clients.is_empty() {
        step(5, t("later"));
    } else {
        step(5, &tf("connecting", &[("client", &join_labels(&clients))]));
        for client in &clients {
            crate::mcp_install(client, false, false)?;
        }
    }
    let spec = crate::sites::load(home, &site)?;
    let example = spec
        .operations
        .iter()
        .find(|op| operation_effect(op, &BTreeMap::new()) == Effect::Read)
        .map(|op| {
            tf(
                "example_read",
                &[("site", &site), ("desc", &mid_sentence(&op.desc))],
            )
        })
        .unwrap_or_default();
    eprintln!();
    eprintln!("{}", tf("done", &[("site", &site), ("example", &example)]));
    if spec
        .operations
        .iter()
        .any(|op| operation_effect(op, &BTreeMap::new()) != Effect::Read)
    {
        eprintln!("{}", t("approve_note"));
    }
    Ok(ExitCode::SUCCESS)
}

fn name_width(spec: &Spec) -> usize {
    spec.operations
        .iter()
        .map(|op| op.name.len())
        .max()
        .unwrap_or(0)
        .max(12)
}

fn mid_sentence(desc: &str) -> String {
    if crate::text::lang() != "en" {
        return desc.to_owned();
    }
    mid_sentence_en(desc)
}

fn mid_sentence_en(desc: &str) -> String {
    let mut chars = desc.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second))
            if first.is_ascii_uppercase() && second.is_ascii_lowercase() =>
        {
            first.to_ascii_lowercase().to_string() + &desc[1..]
        }
        _ => desc.to_owned(),
    }
}

fn step(n: u8, text: &str) {
    eprintln!("{}  {text}", tf("step", &[("n", &n.to_string())]));
}

/// What the GraphQL endpoints the page or the recording named offer, said
/// under the current step.
async fn introspect(
    home: &Path,
    endpoints: &[String],
) -> AppResult<Vec<itda_prep::graphql::Schema>> {
    if endpoints.is_empty() {
        return Ok(Vec::new());
    }
    let schemas = crate::introspect::schemas(&crate::open(home)?.net, endpoints).await;
    crate::introspect::say(&schemas, "    ");
    Ok(schemas)
}

/// Fetch the page as itda; `Err` says why that is not enough.
async fn probe(home: &Path, url: &str) -> AppResult<Result<itda_prep::Page, Fallback>> {
    let net = crate::open(home)?.net;
    let request = itda_net::Request {
        limit: 8 * 1024 * 1024,
        ..itda_net::Request::read(url)
    };
    let response = match net.fetch(request).await {
        Ok(response) => response,
        Err(error)
            if matches!(
                error.public_code.as_str(),
                "blocked_by_site" | "site_rate_limited"
            ) =>
        {
            return Ok(Err(Fallback::Blocked));
        }
        Err(error) => return Err(error),
    };
    if let Some(error) = crate::site_refusal(url, &response) {
        eprintln!("    {}", error.msg);
        return Ok(Err(if error.public_code == "auth_required" {
            Fallback::SignIn
        } else {
            Fallback::Blocked
        }));
    }
    if matches!(response.status, 401 | 403) {
        return Ok(Err(Fallback::SignIn));
    }
    if !response.successful() {
        return Ok(Err(Fallback::Blocked));
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
    if evidence::script_shell(&html) {
        return Ok(Err(Fallback::Scripted));
    }
    if signed_in_form && response.url.to_lowercase().contains("login") {
        return Ok(Err(Fallback::SignIn));
    }
    Ok(Ok(itda_prep::Page {
        url: response.url.clone(),
        html,
    }))
}

/// "a list of 42 records, 2 forms, a feed": what the page offers, in words.
fn describe_page(page: &itda_prep::Page) -> String {
    let mut parts = Vec::new();
    if let Some(extraction) = evidence::infer_extraction(&page.html, &page.url)
        && let Some(value) = extraction.extract(&page.html, &page.url)
    {
        let n = value["items"].as_array().map(Vec::len).unwrap_or(0);
        parts.push(tf("found_list", &[("n", &n.to_string())]));
    } else if !itda_html::extract_ssr_map(&page.html).is_empty() {
        parts.push(t("found_data").to_owned());
    }
    let forms = evidence::forms(&page.html, &page.url)
        .iter()
        .filter(|f| !f.password)
        .count();
    if forms == 1 {
        parts.push(t("found_form").to_owned());
    } else if forms > 1 {
        parts.push(tf("found_forms", &[("n", &forms.to_string())]));
    }
    if !evidence::feeds(&page.html, &page.url).is_empty() {
        parts.push(t("found_feed").to_owned());
    }
    if evidence::openapi_link(&page.html, &page.url).is_some() {
        parts.push(t("found_description").to_owned());
    }
    if parts.is_empty() {
        parts.push(t("found_title").to_owned());
    }
    parts.join(", ")
}

fn list_actions(spec: &Spec) {
    let width = name_width(spec);
    for op in &spec.operations {
        let read = operation_effect(op, &BTreeMap::new()) == Effect::Read;
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
        let mut line = format!(
            "    {} {:<width$} {}",
            if read { t("read") } else { t("change") },
            op.name,
            op.desc
        );
        if !inputs.is_empty() {
            line.push_str(&format!(
                "  ({})",
                tf("inputs", &[("list", &inputs.join(", "))])
            ));
        }
        if !read {
            line.push_str(&format!("  {}", t("change_note")));
        }
        eprintln!("{line}");
    }
}

/// The definition without the named operations, with its comments kept.
fn without(yaml: &str, names: &[String]) -> AppResult<String> {
    let (header, body): (Vec<&str>, Vec<&str>) = yaml.lines().partition(|l| l.starts_with('#'));
    let mut doc: serde_norway::Value =
        serde_norway::from_str(&body.join("\n")).map_err(|_| AppError::api("internal", 500))?;
    if let Some(serde_norway::Value::Sequence(ops)) = doc.get_mut("operations") {
        ops.retain(|op| {
            op.get("name")
                .and_then(serde_norway::Value::as_str)
                .is_none_or(|name| !names.iter().any(|n| n == name))
        });
    }
    let rest = serde_norway::to_string(&doc).map_err(|_| AppError::api("internal", 500))?;
    Ok(format!("{}\n{rest}", header.join("\n")))
}

/// A yes or no in the terminal; `default` answers an empty line.
pub(crate) fn ask(question: &str, default: bool) -> AppResult<bool> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return Ok(default);
    }
    eprint!("    {question}");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    stdin.lock().read_line(&mut answer)?;
    Ok(match answer.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "y" | "yes" | "ㅇ" | "네" | "예" => true,
        _ => false,
    })
}

/// After a ready-made site was added from the welcome prompt: connect the
/// agent apps found on this computer, the way `itda setup` ends.
pub(crate) fn connect_detected() -> AppResult<()> {
    let clients = choose_clients()?;
    if clients.is_empty() {
        eprintln!("{}", t("later"));
        return Ok(());
    }
    eprintln!(
        "{}",
        tf("connecting", &[("client", &join_labels(&clients))])
    );
    for client in &clients {
        crate::mcp_install(client, false, false)?;
    }
    Ok(())
}

fn join_labels(clients: &[String]) -> String {
    let labels: Vec<&str> = clients.iter().map(|c| client_label(c)).collect();
    match labels.split_last() {
        Some((last, rest)) if !rest.is_empty() => {
            format!("{}{}{last}", rest.join(", "), t("and"))
        }
        _ => labels.concat(),
    }
}

/// The agent apps found on this computer, after one yes; the numbered list
/// only when none is found.
fn choose_clients() -> AppResult<Vec<String>> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return Ok(Vec::new());
    }
    let user_home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let cwd = std::env::current_dir().unwrap_or_default();
    let found: Vec<String> = crate::install::detected(&user_home, &cwd)
        .into_iter()
        .map(str::to_owned)
        .collect();
    if !found.is_empty() {
        let list = join_labels(&found);
        return Ok(if ask(&tf("ask_connect", &[("list", &list)]), true)? {
            found
        } else {
            Vec::new()
        });
    }
    Ok(match choose_client()?.as_str() {
        "none" => Vec::new(),
        client => vec![client.to_owned()],
    })
}

fn choose_client() -> AppResult<String> {
    let stdin = std::io::stdin();
    eprint!("    {}", t("ask_client"));
    std::io::stderr().flush()?;
    let mut answer = String::new();
    stdin.lock().read_line(&mut answer)?;
    Ok(match answer.trim() {
        "1" => "claude-desktop",
        "2" => "claude-code",
        "3" => "cursor",
        "4" => "vscode",
        "5" => "codex",
        _ => "none",
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::mid_sentence_en;

    #[test]
    fn lowercases_a_leading_word_but_not_an_acronym() {
        assert_eq!(
            mid_sentence_en("List what Hacker News shows"),
            "list what Hacker News shows"
        );
        assert_eq!(mid_sentence_en("RSS feed"), "RSS feed");
        assert_eq!(mid_sentence_en("목록 읽기"), "목록 읽기");
        assert_eq!(mid_sentence_en(""), "");
    }
}
