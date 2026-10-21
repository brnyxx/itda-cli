//! `itda record`: the person uses a site in their own browser while itda
//! watches the requests, then drafts a definition from them.
use itda_types::{
    error::{AppResult, usage},
    url::validate_url,
};
use serde_json::Value;
use std::time::Duration;

/// What a recording session leaves behind: the HAR document and the cookies
/// the browser held for the site when it closed.
pub struct Recording {
    pub har: Value,
    pub cookies: Vec<itda_browser::Cookie>,
}

/// Open the browser at `page` and record until the person presses Enter or
/// closes the window. Blocks; run it off the async runtime.
pub fn browser(page: &str) -> AppResult<Recording> {
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
    eprintln!("Recorded {count} calls.");
    Ok(Recording { har, cookies })
}

/// The value a recorded request sent in `header` to `base_url`, without
/// `prefix`: the token the drafted `auth` entry refers to. Never printed.
pub fn header_value(har: &Value, base_url: &str, header: &str, prefix: &str) -> Option<String> {
    har["log"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry["request"]["url"]
                .as_str()
                .is_some_and(|url| url.starts_with(base_url))
        })
        .find_map(|entry| {
            entry["request"]["headers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|h| {
                    h["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(header))
                })
                .and_then(|h| h["value"].as_str())
                .map(|value| {
                    let value = value.trim();
                    if !prefix.is_empty()
                        && value.len() >= prefix.len()
                        && value[..prefix.len()].eq_ignore_ascii_case(prefix)
                    {
                        value[prefix.len()..].trim().to_owned()
                    } else {
                        value.to_owned()
                    }
                })
                .filter(|value| !value.is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::header_value;
    use serde_json::json;

    #[test]
    fn the_token_comes_from_the_site_without_its_prefix() {
        let har = json!({"log": {"entries": [
            {"request": {"url": "https://other.example/x", "headers": [{"name": "Authorization", "value": "Bearer elsewhere"}]}},
            {"request": {"url": "https://app.example/api/me", "headers": [{"name": "authorization", "value": "Bearer abc.def"}]}},
        ]}});
        assert_eq!(
            header_value(&har, "https://app.example", "Authorization", "Bearer ").as_deref(),
            Some("abc.def")
        );
        assert_eq!(
            header_value(&har, "https://app.example", "X-Api-Key", "").as_deref(),
            None
        );
    }
}
