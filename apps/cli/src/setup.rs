//! `itda setup <url>`: one command from a page address to an agent that can
//! use the site. The steps live in `itda_setup`; this asks the questions,
//! says what happens in the person's language, and connects an agent at
//! the end.
use crate::text::{client_label, t, tf};
use itda_policy::{Effect, operation_effect};
use itda_setup::{Answers, Fallback, Finished, Progress, Stopped, Template, check};
use itda_spec::Spec;
use itda_types::error::{AppResult, usage};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal, Write},
    path::Path,
    process::ExitCode,
    sync::mpsc,
};

pub struct Flags {
    pub url: String,
    pub name: Option<String>,
    /// `Some(true)` records, `Some(false)` never opens the browser, `None` asks.
    pub record: Option<bool>,
    pub template: Template,
    /// Watch the page load when it shows nothing, but never ask to record.
    pub watch_only: bool,
    pub client: Option<String>,
    pub yes: bool,
}

/// The questions, asked in the terminal.
struct Terminal {
    record: Option<bool>,
    watch_only: bool,
    yes: bool,
}

impl Answers for Terminal {
    fn record_anyway(&mut self) -> AppResult<bool> {
        if self.record.is_some() || self.watch_only {
            return Ok(false);
        }
        ask(t("ask_record_anyway"), false)
    }

    fn record(&mut self, why: Fallback) -> AppResult<bool> {
        if self.record == Some(false) {
            return Err(usage(t("no_record_allowed"), ""));
        }
        if self.watch_only {
            return Err(usage(tf("watch_only_stop", &[("why", why_words(why))]), ""));
        }
        ask(t("ask_record"), true)
    }

    fn recording_done(&mut self) -> Option<mpsc::Receiver<()>> {
        Some(enter_pressed())
    }

    fn replace(&mut self, site: &str) -> AppResult<bool> {
        if self.yes {
            return Ok(true);
        }
        ask(&tf("exists", &[("site", site)]), false)
    }

    fn can_keep_credentials(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn keep_credential(&mut self, site: &str, what: &str) -> AppResult<bool> {
        ask_keep(site, what)
    }
}

pub async fn run(home: &Path, flags: Flags) -> AppResult<ExitCode> {
    let interactive = std::io::stdin().is_terminal();
    let record_answered = flags.record.is_some() || flags.watch_only;
    if !interactive && (!record_answered || flags.client.is_none()) {
        return Err(usage(t("no_terminal"), ""));
    }
    let ctx = crate::open(home)?.context(home);
    let mut answers = Terminal {
        record: flags.record,
        watch_only: flags.watch_only,
        yes: flags.yes,
    };
    let mut width = 12;
    let finished = itda_setup::run(
        &ctx,
        itda_setup::Request {
            url: flags.url.clone(),
            name: flags.name.clone(),
            record: flags.record == Some(true),
            template: flags.template.clone(),
            watch: flags.record.is_none(),
        },
        &mut answers,
        &mut |progress| say(progress, &mut width),
    )
    .await?;
    let done = match finished {
        Finished::Done(done) => done,
        Finished::Stopped(Stopped::NoRecord { .. }) => return Ok(ExitCode::FAILURE),
        Finished::Stopped(Stopped::Exists(site)) => {
            eprintln!("    {}", tf("exists_kept", &[("site", &site)]));
            return Ok(ExitCode::FAILURE);
        }
        Finished::Stopped(Stopped::NothingWorks) => {
            return Err(usage(tf("nothing_works", &[("url", &flags.url)]), ""));
        }
    };
    let site = done.site;

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
    let spec = done.spec;
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

/// Each step, in words, as it happens. `width` is the column the action
/// names take, set when the draft is shown.
fn say(progress: Progress<'_>, width: &mut usize) {
    match progress {
        Progress::Looking { host } => step(1, &tf("looking", &[("host", host)])),
        Progress::Recognised(found) => {
            let key = if found.confirmed {
                "recognised"
            } else {
                "template_unconfirmed"
            };
            eprintln!(
                "    {}",
                tf(key, &[("software", &found.title), ("base", &found.base)])
            );
        }
        Progress::PageFetched(summary) => {
            eprintln!("    {}", t("fetched_page"));
            eprintln!("    {}", tf("found", &[("what", &describe_page(summary))]));
        }
        Progress::SchemaAsked { url } => {
            eprintln!("    {}", tf("asked_schema", &[("url", url)]));
        }
        Progress::Described(described) => {
            eprintln!("    {}", crate::text::described(described));
        }
        Progress::Refused(refused) => eprintln!("    {}", crate::refusal_words(refused).msg),
        Progress::Watching { host } => eprintln!("    {}", tf("watching", &[("host", host)])),
        Progress::Watched { calls, reads } => {
            let calls = calls.to_string();
            let line = if reads == 0 {
                tf("watched_nothing", &[("n", &calls)])
            } else {
                tf("watched", &[("n", &calls), ("k", &reads.to_string())])
            };
            eprintln!("    {line}");
        }
        Progress::NeedsRecord(why) => {
            eprintln!("    {}", tf("needs_record", &[("why", why_words(why))]));
        }
        Progress::Recording(progress) => crate::say_recording(&progress),
        Progress::Drafted(spec) => {
            step(2, t("actions_heading"));
            *width = name_width(spec);
            list_actions(spec, *width);
        }
        Progress::Trying(spec) => {
            *width = name_width(spec);
            step(3, t("trying"));
        }
        Progress::Tried { spec, outcome } => eprintln!("    {}", tried(spec, outcome, *width)),
        Progress::Saved { site } => step(4, &tf("saved", &[("site", site)])),
        Progress::CredentialKept {
            name,
            site,
            account,
        } => crate::say_credential_kept(name, site, account),
    }
}

/// One line per read tried: kept, skipped, or removed with why.
fn tried(spec: &Spec, outcome: &check::Outcome, width: usize) -> String {
    let name = &outcome.action;
    let op = spec.op(name).ok();
    let read = op.is_none_or(|op| operation_effect(op, &BTreeMap::new()) == Effect::Read);
    match outcome.status.as_str() {
        "ok" => match outcome.records {
            Some(n) => format!(
                "✓ {name:<width$} {}",
                tf("ok_records", &[("n", &n.to_string())])
            ),
            None => format!("✓ {name:<width$} {}", t("ok")),
        },
        "skipped" if !read => format!("· {name:<width$} {}", t("skipped_change")),
        "skipped" => format!(
            "· {name:<width$} {}",
            tf(
                "skipped_needs",
                &[(
                    "list",
                    &op.map(check::needed_inputs).unwrap_or_default().join(", ")
                )]
            )
        ),
        code => {
            let why = match (code, outcome.retry_after_s) {
                ("response_mismatch", _) => t("why_mismatch").to_owned(),
                ("auth_required", _) => t("why_auth").to_owned(),
                ("not_found", _) => t("why_not_found").to_owned(),
                ("blocked_by_site", _) => t("why_blocked").to_owned(),
                ("site_rate_limited", Some(seconds)) => {
                    tf("why_wait", &[("n", &seconds.to_string())])
                }
                ("site_rate_limited", None) => t("why_rate_limited").to_owned(),
                _ => outcome.detail.clone(),
            };
            format!("✗ {name:<width$} {}", tf("removed", &[("why", &why)]))
        }
    }
}

/// Why the page was not enough, in a few words.
fn why_words(why: Fallback) -> &'static str {
    match why {
        Fallback::Blocked => t("why_blocked"),
        Fallback::SignIn => t("why_signin"),
        Fallback::Scripted => t("why_scripted"),
    }
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

/// "a list of 42 records, 2 forms, a feed": what the page offers, in words.
fn describe_page(summary: &itda_setup::PageSummary) -> String {
    let mut parts = Vec::new();
    if summary.main {
        parts.push(t("found_main").to_owned());
    }
    if let Some(n) = summary.records {
        parts.push(tf("found_list", &[("n", &n.to_string())]));
    } else if summary.embedded_data {
        parts.push(t("found_data").to_owned());
    }
    if summary.forms == 1 {
        parts.push(t("found_form").to_owned());
    } else if summary.forms > 1 {
        parts.push(tf("found_forms", &[("n", &summary.forms.to_string())]));
    }
    if summary.feed {
        parts.push(t("found_feed").to_owned());
    }
    if summary.description_link {
        parts.push(t("found_description").to_owned());
    }
    if parts.is_empty() {
        parts.push(t("found_title").to_owned());
    }
    parts.join(", ")
}

fn list_actions(spec: &Spec, width: usize) {
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

/// Keep `what` in the keyring? The same question `itda record --install`
/// asks; values are never shown.
pub(crate) fn ask_keep(site: &str, what: &str) -> AppResult<bool> {
    eprint!(
        "Keep {what} as {site}/{} in the system keyring? Agents only see the account id. [y/N] ",
        itda_runtime::DEFAULT_ACCOUNT
    );
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

/// Fires once when the person presses Enter.
pub(crate) fn enter_pressed() -> mpsc::Receiver<()> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        if std::io::stdin()
            .lock()
            .read_line(&mut line)
            .is_ok_and(|read| read > 0)
        {
            let _ = sender.send(());
        }
    });
    receiver
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
