//! `itda refresh <site>`: draft the site again from the sources it was set
//! up from, show what would change in plain words, write it after a yes,
//! then check the site. The steps live in `itda_setup::refresh`.
use crate::text::{t, tf};
use itda_setup::{Progress, check, refresh};
use itda_types::error::AppResult;
use std::{path::Path, process::ExitCode};

pub async fn run(home: &Path, site: &str, yes: bool) -> AppResult<ExitCode> {
    let ctx = crate::open(home)?.context(home);
    let kind = refresh::sources(home, site)
        .map(|sources| sources.kind())
        .unwrap_or("pages");
    eprintln!(
        "{}",
        tf(
            "refresh_from",
            &[("site", site), ("what", t(&format!("from_{kind}")))]
        )
    );
    let plan = refresh::plan(&ctx, site, &mut say).await?;
    let before = check::verdicts(&plan.outcomes);
    if let Some(verdict) = before.first() {
        eprintln!(
            "    {}",
            tf(
                "refresh_before",
                &[
                    ("verdict", verdict.verdict.as_str()),
                    ("hint", &verdict.hint)
                ]
            )
        );
    }
    for note in &plan.notes {
        eprintln!("    note: {note}");
    }
    for problem in &plan.problems {
        eprintln!("    problem: {problem}");
    }
    if !plan.changes.is_empty() {
        eprintln!("{}", t("refresh_heading"));
    }
    let width = plan
        .changes
        .iter()
        .map(|change| change.name().len())
        .max()
        .unwrap_or(0);
    for change in &plan.changes {
        eprintln!("    {}", line(change, width));
    }
    if !plan.changes_anything() {
        eprintln!("{}", tf("refresh_same", &[("site", site)]));
        return Ok(ExitCode::SUCCESS);
    }
    if !yes && !crate::setup::ask(&tf("ask_refresh", &[("site", site)]), false)? {
        eprintln!("{}", tf("refresh_kept", &[("site", site)]));
        return Ok(ExitCode::SUCCESS);
    }
    refresh::apply(home, &plan)?;
    // What the user may have reviewed is no longer what would be sent.
    ctx.approvals.invalidate(Some(site), None).await?;
    eprintln!("{}", tf("refresh_written", &[("site", site)]));
    let outcomes = check::run(&ctx.runtime, std::slice::from_ref(&plan.spec), None).await;
    let verdicts = check::verdicts(&outcomes);
    check::remember(home, &verdicts)?;
    crate::say_check(&outcomes, &verdicts);
    Ok(ExitCode::from(crate::output::exit_code(
        check::exit_status(&outcomes),
    )))
}

/// One line of the diff: what happens to the action, and why.
fn line(change: &refresh::Change, width: usize) -> String {
    match change {
        refresh::Change::Added { name, desc } => {
            format!(
                "+ {name:<width$}  {}",
                tf("refresh_added", &[("desc", desc)])
            )
        }
        refresh::Change::Removed { name, why } => {
            let why = match why.as_str() {
                "response_mismatch" => t("why_mismatch"),
                "not_found" => t("why_not_found"),
                "auth_required" => t("why_auth"),
                other => other,
            };
            format!(
                "- {name:<width$}  {}",
                tf("refresh_removed", &[("why", why)])
            )
        }
        refresh::Change::Changed { name, fields } => format!(
            "~ {name:<width$}  {}",
            tf("refresh_changed", &[("fields", &fields.join(", "))])
        ),
        refresh::Change::Edited { name } => {
            format!("= {name:<width$}  {}", t("refresh_edited"))
        }
    }
}

/// What the steps say on the way, in the person's language.
fn say(progress: Progress<'_>) {
    match progress {
        Progress::Looking { host } => eprintln!("    {}", tf("looking", &[("host", host)])),
        Progress::Recognised(found) => eprintln!(
            "    {}",
            tf(
                "recognised",
                &[("software", &found.title), ("base", &found.base)]
            )
        ),
        Progress::SchemaAsked { url } => {
            eprintln!("    {}", tf("asked_schema", &[("url", url)]));
        }
        Progress::Described(described) => {
            eprintln!("    {}", crate::text::described(described));
        }
        Progress::Refused(refused) => eprintln!("    {}", crate::refusal_words(refused).msg),
        _ => {}
    }
}
