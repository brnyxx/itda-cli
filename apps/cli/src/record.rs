//! `itda record`: the person uses a site in their own browser while itda
//! watches the requests, then drafts a definition from them.
use itda_types::{
    error::{AppResult, usage},
    url::validate_url,
};
use serde_json::Value;
use std::time::Duration;

/// Open the browser at `page` and record until the person presses Enter or
/// closes the window. Returns the recording as a HAR document. Blocks; run
/// it off the async runtime.
pub fn browser(page: &str) -> AppResult<Value> {
    let page = validate_url(page)?;
    let mut recorder = itda_browser::Recorder::open(&itda_browser::find()?, &page)?;
    eprintln!(
        "Recording {}. Use the site in the window that just opened: sign in, search, open things, post a form.",
        page.host_str().unwrap_or_default()
    );
    eprintln!("Press Enter here, or close the window, when you are done.");
    let enter = crate::login::enter_pressed();
    let mut shown = 0;
    loop {
        recorder.poll(Duration::from_millis(500))?;
        if recorder.len() >= shown + 25 {
            shown = recorder.len();
            eprintln!("  {shown} calls so far");
        }
        if enter.try_recv().is_ok() || !recorder.is_open() {
            break;
        }
    }
    // Replies to the last body fetches may still be on the way.
    recorder.poll(Duration::from_millis(750))?;
    let count = recorder.len();
    let har = recorder.finish();
    if count == 0 {
        return Err(usage(
            "Nothing was recorded",
            "Use the site in the window before pressing Enter; itda records what the pages ask the server for.",
        ));
    }
    eprintln!("Recorded {count} calls.");
    Ok(har)
}
