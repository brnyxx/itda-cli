//! Frames: a portal whose page is a shell and whose content lives in
//! `<iframe>`s (or an older frameset) on the same origin. The frame pages
//! are fetched as itda, with the same pacing and the same session as the
//! page, and drafted as pages of the site, each under its own address, since
//! that is what a request to the server looks like. A frame on another
//! origin is only named, never fetched.
use crate::login::Session;
use crate::{Refusal, Refused};
use itda_html::evidence;
use itda_net::{Network, Request};
use itda_prep::Page;
use itda_types::error::AppResult;

/// How many frame pages are followed at most: a portal has a menu, a body
/// and perhaps a footer, not a wall of them.
pub const MAX_FRAMES: usize = 4;
/// How deep frames in frames are followed.
pub const MAX_DEPTH: usize = 2;

/// What following a page's frames gave: the frame pages, and the address of
/// each one followed (its path, with a query the frame carried, without the
/// session), in the order they were read.
#[derive(Default)]
pub struct Framed {
    pub pages: Vec<Page>,
    pub followed: Vec<String>,
    /// A same-origin frame answered with a redirect to a sign-in page, so it
    /// could not be read without a session: the gate's view of that redirect
    /// (the frame's path and the sign-in page it leads to). The caller signs
    /// in and follows again with the session, the way the top-level page
    /// would.
    pub sign_in: Option<Refused>,
}

impl Framed {
    /// The paths of the frames that answered with a sign-in, for the draft's
    /// note: the frame was shown, led to a sign-in and so was not read.
    pub fn sign_in_paths(&self) -> Vec<String> {
        match &self.sign_in {
            Some(Refused {
                kind: Refusal::SignInRedirect { path, .. },
                ..
            }) => vec![path.clone()],
            _ => Vec::new(),
        }
    }
}

/// One frame page as itda: the page itself, or the gate's view of a redirect
/// to a sign-in that itda reads only with a session.
enum Fetched {
    Page(Page),
    SignIn(Refused),
}

/// Whether the page is a shell whose content is in frames on its own
/// origin: it has at least one such frame and nothing of its own to read.
pub fn is_shell(page: &Page) -> bool {
    crate::summarize(page).bare() && has_own_frames(&page.html, &page.url)
}

/// Whether the page shows another page of its own origin in a frame.
pub fn has_own_frames(html: &str, url: &str) -> bool {
    evidence::frames(html, url).iter().any(|f| f.same_origin)
}

/// Fetch the same-origin frames of `page` when it is a shell, [`MAX_DEPTH`]
/// deep and [`MAX_FRAMES`] at most, each address once, with `session` the
/// way the page was fetched. A frame that does not answer with a page is
/// skipped; the page itself is still drafted.
pub async fn follow(net: &Network, page: &Page, session: Option<&Session>) -> AppResult<Framed> {
    let mut framed = Framed::default();
    if !is_shell(page) {
        return Ok(framed);
    }
    let Ok(origin) = url::Url::parse(&page.url) else {
        return Ok(framed);
    };
    let session_key = session
        .and_then(|s| s.query.as_ref())
        .map(|(key, _)| key.as_str());
    let mut seen = vec![page.url.clone()];
    let mut queue: Vec<(String, usize)> = evidence::frames(&page.html, &page.url)
        .into_iter()
        .filter(|f| f.same_origin)
        .map(|f| (f.url, 1))
        .collect();
    while let Some((url, depth)) = (!queue.is_empty()).then(|| queue.remove(0)) {
        if framed.pages.len() == MAX_FRAMES {
            break;
        }
        let shown = match session_key {
            Some(key) => itda_types::url::without_query_key(&url, key),
            None => url.clone(),
        };
        if seen.contains(&shown) {
            continue;
        }
        seen.push(shown.clone());
        let frame = match fetch(net, &url, session).await {
            Some(Fetched::Page(page)) => page,
            Some(Fetched::SignIn(refused)) => {
                // The frame wants a sign-in; note it once so the caller can
                // sign in and follow again with the session.
                framed.sign_in.get_or_insert(refused);
                continue;
            }
            None => continue,
        };
        let frame = Page {
            url: match session_key {
                Some(key) => itda_types::url::without_query_key(&frame.url, key),
                None => frame.url,
            },
            html: frame.html,
        };
        if seen.iter().filter(|s| **s == frame.url).count() > 1 || frame.url == page.url {
            // The frame answered with a page already read (a redirect).
            continue;
        }
        if frame.url != shown {
            seen.push(frame.url.clone());
        }
        if let Ok(address) = url::Url::parse(&frame.url)
            && address.origin() == origin.origin()
        {
            framed.followed.push(match address.query() {
                Some(query) => format!("{}?{query}", address.path()),
                None => address.path().to_owned(),
            });
        }
        if depth < MAX_DEPTH {
            queue.extend(
                evidence::frames(&frame.html, &frame.url)
                    .into_iter()
                    .filter(|f| f.same_origin)
                    .map(|f| (f.url, depth + 1)),
            );
        }
        framed.pages.push(frame);
    }
    Ok(framed)
}

/// One frame page as itda: a bounded GET with the session, when there is
/// one, that answers with HTML on the same origin. A frame that answers with
/// a redirect to a sign-in page is reported as [`Fetched::SignIn`], the same
/// detection the top-level page uses, so the caller can sign in and read it.
/// Anything else is `None`.
async fn fetch(net: &Network, url: &str, session: Option<&Session>) -> Option<Fetched> {
    let Ok(parsed) = url::Url::parse(url) else {
        return None;
    };
    let address = match session.and_then(|s| s.query.as_ref()) {
        Some((key, value)) => itda_types::url::with_query_pair(url, key, value),
        None => url.to_owned(),
    };
    let mut request = Request {
        limit: 8 * 1024 * 1024,
        ..Request::read(address)
    };
    if let Some(cookies) = session
        .map(|s| s.cookies.as_str())
        .filter(|c| !c.is_empty())
    {
        request.headers.push(("cookie".into(), cookies.to_owned()));
    }
    let response = net.fetch(request).await.ok()?;
    // The net layer does not follow a redirect to a sign-in (its path is a
    // credential path); the gate reads it here so the caller treats the
    // frame as the top-level page would.
    if let Some(refused) = crate::refusal(url, &response) {
        return matches!(refused.kind, Refusal::SignInRedirect { .. })
            .then_some(Fetched::SignIn(refused));
    }
    if !response.successful() || !response.header("content-type").contains("html") {
        return None;
    }
    let landed = url::Url::parse(&response.url).ok()?;
    if landed.origin() != parsed.origin() {
        return None;
    }
    Some(Fetched::Page(Page {
        url: response.url,
        html: String::from_utf8_lossy(&response.bytes).into_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::is_shell;
    use itda_prep::Page;

    #[test]
    fn a_shell_has_a_frame_of_its_own_and_nothing_else_to_read() {
        let shell = Page {
            url: "https://portal.example/".into(),
            html: "<title>Portal</title><iframe src='/frame/board'></iframe>".into(),
        };
        assert!(is_shell(&shell));
        let elsewhere = Page {
            url: "https://portal.example/".into(),
            html: "<title>Portal</title><iframe src='https://cdn.example/w'></iframe>".into(),
        };
        assert!(!is_shell(&elsewhere));
        let board = Page {
            url: "https://portal.example/".into(),
            html: "<title>Portal</title><iframe src='/frame/x'></iframe>
                <table><thead><tr><th>No</th><th>Title</th></tr></thead>
                <tr><td>1</td><td><a href='/n/1'>A</a></td></tr>
                <tr><td>2</td><td><a href='/n/2'>B</a></td></tr>
                <tr><td>3</td><td><a href='/n/3'>C</a></td></tr></table>"
                .into(),
        };
        assert!(!is_shell(&board), "a page with records is read as itself");
    }
}
