//! Recording: the person uses a site in their own browser while itda watches
//! the requests, so a definition can be drafted from them.
use itda_types::{
    error::{AppResult, usage},
    url::validate_url,
};
use serde_json::Value;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

/// The network has to be quiet for this long before a watched load counts
/// as finished...
const QUIET: Duration = Duration::from_secs(2);
/// ...and this is the longest one wait lasts, quiet or not.
const WAIT_CAP: Duration = Duration::from_secs(10);

/// What a recording session leaves behind: the HAR document and the cookies
/// the browser held for the site when it closed.
pub struct Recording {
    pub har: Value,
    pub cookies: Vec<itda_browser::Cookie>,
}

/// What the recording says while it runs.
#[derive(Debug)]
pub enum Progress {
    /// The window is open: use the site there.
    Opened { host: String },
    /// Every 25 calls.
    Calls(usize),
    /// The window closed or the person said so.
    Finished(usize),
}

/// Open the browser at `page` and record until `done` fires or the person
/// closes the window. Blocks; run it off the async runtime.
pub fn browser(
    page: &str,
    done: Option<&mpsc::Receiver<()>>,
    report: &mut dyn FnMut(Progress),
) -> AppResult<Recording> {
    let page = validate_url(page)?;
    let mut recorder = itda_browser::Recorder::open(&itda_browser::find()?, &page)?;
    report(Progress::Opened {
        host: page.host_str().unwrap_or_default().to_owned(),
    });
    let mut shown = 0;
    loop {
        recorder.poll(Duration::from_millis(500))?;
        if recorder.len() >= shown + 25 {
            shown = recorder.len();
            report(Progress::Calls(shown));
        }
        if done.is_some_and(|done| done.try_recv().is_ok()) || !recorder.is_open() {
            break;
        }
    }
    // Replies to the last body fetches may still be on the way.
    recorder.poll(Duration::from_millis(750))?;
    let count = recorder.len();
    let cookies = if recorder.is_open() {
        recorder.cookies(&page).unwrap_or_default()
    } else {
        Vec::new()
    };
    let har = recorder.finish();
    if count == 0 {
        return Err(usage(
            "Nothing was recorded",
            "Use the site in the window before pressing Enter; itda records what the pages ask the server for.",
        ));
    }
    report(Progress::Finished(count));
    Ok(Recording { har, cookies })
}

/// Open `browser` at `page`, watch what the page loads on its own, scroll
/// once to the bottom, watch what that loads, and close the window: the
/// calls a page drawn by scripts makes without anyone clicking. Each wait
/// ends when the network has been quiet for two seconds, or after ten.
/// Nothing is clicked or typed, and no cookie is read back. Blocks; run it
/// off the async runtime. The recording may be empty.
pub fn watch(browser: &std::path::Path, page: &str) -> AppResult<Recording> {
    let page = validate_url(page)?;
    let mut recorder = itda_browser::Recorder::open(browser, &page)?;
    settle(&mut recorder)?;
    if recorder.is_open() {
        recorder.scroll_to_bottom()?;
        settle(&mut recorder)?;
    }
    // Replies to the last body fetches may still be on the way.
    recorder.poll(Duration::from_millis(750))?;
    Ok(Recording {
        har: recorder.finish(),
        cookies: Vec::new(),
    })
}

/// Handle what the browser sends until the network has been quiet for
/// [`QUIET`], the window closed, or [`WAIT_CAP`] passed.
fn settle(recorder: &mut itda_browser::Recorder) -> AppResult<()> {
    let started = Instant::now();
    while started.elapsed() < WAIT_CAP {
        recorder.poll(Duration::from_millis(250))?;
        if !recorder.is_open() || recorder.quiet_for().is_some_and(|quiet| quiet >= QUIET) {
            break;
        }
    }
    Ok(())
}
