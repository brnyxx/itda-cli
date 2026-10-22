//! The API description a site publishes, for `itda draft page` and `itda
//! setup`: the one a page links to, or the one a well-known path answers
//! with. Fetched as itda, by GET, under a size cap; whatever is not there is
//! simply not there, so nothing here fails the page draft.
use itda_html::evidence;
use itda_net::{Network, Request};
use itda_prep::{DESCRIPTION_PATHS, Description, Page};

/// Descriptions are read whole; huge ones are better drafted on their own.
const LIMIT: usize = 4 * 1024 * 1024;

/// The description the pages link to, else the first well-known path on the
/// first page's origin that answers with one. A linked document is returned
/// even when it does not parse, so the draft can say so.
pub(crate) async fn find(net: &Network, pages: &[Page]) -> Option<Description> {
    for page in pages {
        if let Some(link) = evidence::openapi_link(&page.html, &page.url) {
            return fetch(net, &link).await;
        }
    }
    let mut origin = url::Url::parse(&pages.first()?.url).ok()?;
    origin.set_query(None);
    origin.set_fragment(None);
    for path in DESCRIPTION_PATHS {
        origin.set_path(path);
        if let Some(found) = fetch(net, origin.as_str()).await
            && itda_prep::is_openapi(&found.bytes)
        {
            return Some(found);
        }
    }
    None
}

async fn fetch(net: &Network, url: &str) -> Option<Description> {
    let request = Request {
        limit: LIMIT,
        ..Request::read(url)
    };
    let response = net.fetch(request).await.ok()?;
    if !response.successful() {
        return None;
    }
    Some(Description {
        url: response.url,
        bytes: response.bytes,
    })
}
