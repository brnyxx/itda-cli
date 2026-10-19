//! `itda login --browser`: the person signs in to a site in their own browser,
//! and itda keeps that site's session cookies.
use itda_spec::Spec;
use itda_types::{
    error::{AppResult, usage},
    url::validate_url,
};
use std::{io::BufRead, sync::mpsc, time::Duration};

/// A signed-in session as a `Cookie` header value, and how many cookies it holds.
pub struct Session {
    pub cookies: String,
    pub count: usize,
}

/// Open the browser at the site's sign-in page and wait until the person is
/// signed in: the sign-in cookie the definition names appears, or they press
/// Enter. Blocks; run it off the async runtime.
pub fn browser(spec: &Spec, page: Option<&str>) -> AppResult<Session> {
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

    let mut window = itda_browser::SignIn::open(&itda_browser::find()?, &page)?;
    eprintln!(
        "Sign in to {} in the browser window that just opened.",
        spec.site.name
    );
    match required {
        Some(name) => eprintln!(
            "itda continues when the {name} cookie appears, or press Enter once you are signed in."
        ),
        None => eprintln!("Press Enter here once you are signed in."),
    }
    let enter = enter_pressed();
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
        if signed_in || enter.try_recv().is_ok() {
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

/// Fires once when the person presses Enter.
fn enter_pressed() -> mpsc::Receiver<()> {
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
