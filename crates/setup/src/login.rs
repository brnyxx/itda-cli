//! Browser sign-in: the person signs in to a site in their own browser, and
//! itda keeps that site's session cookies. Nothing here reads a terminal; the
//! caller says when the person is done.
use crate::Context;
use itda_spec::Spec;
use itda_types::{
    error::{AppResult, usage},
    url::validate_url,
};
use itda_vault::Secret;
use std::{sync::mpsc, time::Duration};

/// A signed-in session as a `Cookie` header value, and how many cookies it holds.
pub struct Session {
    pub cookies: String,
    pub count: usize,
}

/// What the sign-in says while it waits.
pub enum Progress {
    /// The window is open: sign in there.
    Opened { site: String },
    /// The sign-in ends when this cookie appears, or when the person says so.
    WaitsFor { cookie: Option<String> },
}

/// Open the browser at the site's sign-in page and wait until the person is
/// signed in: the sign-in cookie the definition names appears, or `done`
/// fires. Without `done` (no terminal to press Enter in), the definition
/// must name the cookie, or there is nothing to wait for. Blocks; run it off
/// the async runtime.
pub fn browser(
    spec: &Spec,
    page: Option<&str>,
    done: Option<&mpsc::Receiver<()>>,
    report: &mut dyn FnMut(Progress),
) -> AppResult<Session> {
    let auth = spec
        .auth
        .iter()
        .find(|auth| auth.kind == "browser")
        .ok_or_else(|| {
            usage(
                format!("{} has no browser sign-in", spec.site.name),
                "Add an auth entry with kind: browser, or store an API key with `itda login` without --browser.",
            )
        })?;
    let scope = validate_url(if auth.origin.is_empty() {
        spec.site.source_url()
    } else {
        &auth.origin
    })?;
    let page = match page {
        Some(page) => validate_url(page)?,
        None => scope.clone(),
    };
    if !itda_types::url::same_site_origin(&scope, &page) {
        return Err(usage(
            "The sign-in page is on another site",
            format!("Use a page on {}.", scope.origin().ascii_serialization()),
        ));
    }
    let required = auth.value_from.strip_prefix("cookie:");
    if done.is_none() && required.is_none() {
        return Err(usage(
            format!(
                "{}'s definition does not name the cookie that shows a sign-in",
                spec.site.name
            ),
            format!(
                "Run `itda login {} --browser` in a terminal and press Enter once you are signed in.",
                spec.site.name
            ),
        ));
    }

    let mut window = itda_browser::SignIn::open(&itda_browser::find()?, &page)?;
    report(Progress::Opened {
        site: spec.site.name.clone(),
    });
    report(Progress::WaitsFor {
        cookie: required.map(str::to_owned),
    });
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if !window.is_open() {
            return Err(usage(
                "The browser was closed before sign-in finished",
                "Run the command again and leave the window open until itda continues.",
            ));
        }
        let cookies = window.cookies(&scope)?;
        let signed_in = required.is_some_and(|name| cookies.iter().any(|c| c.name == name));
        if signed_in || done.is_some_and(|done| done.try_recv().is_ok()) {
            if cookies.is_empty() {
                return Err(usage(
                    "The browser has no cookies for this site",
                    "Finish signing in before pressing Enter.",
                ));
            }
            if let Some(name) = required
                && !signed_in
            {
                return Err(usage(
                    format!("The {name} cookie that shows a sign-in is missing"),
                    "Finish signing in, then run the command again.",
                ));
            }
            return Ok(Session {
                cookies: itda_browser::header(&cookies),
                count: cookies.len(),
            });
        }
    }
}

/// Keep the session as `site/account` and lift any pause on the site: a new
/// sign-in replaces an expired one, so the account may run again.
pub async fn keep(
    ctx: &Context,
    site: &str,
    account: &str,
    label: &str,
    session: Session,
) -> AppResult<()> {
    ctx.vault.add_account(site, account, label).await?;
    ctx.vault
        .set_secret(
            site,
            account,
            itda_runtime::SESSION_SECRET,
            Secret::new(session.cookies),
        )
        .await?;
    // Approvals were bound to the credential the user saw; a new one needs
    // a new look.
    ctx.approvals.invalidate(Some(site), Some(account)).await?;
    crate::resume(ctx, site, Some(account)).await
}
