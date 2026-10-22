//! The API description a site publishes, for `itda draft page` and `itda
//! setup`: the one a page links to, or the one `/openapi.json` answers with.
//! Fetched as itda, by GET, under a size cap; whatever is not there is simply
//! not there, so nothing here fails the page draft.
use itda_html::evidence;
use itda_net::{Network, Request};
use itda_prep::{DESCRIPTION_PATH, Description, Page};

/// Descriptions are read whole; huge ones are better drafted on their own.
const LIMIT: usize = 4 * 1024 * 1024;

/// The description the pages link to, else the one `/openapi.json` on the
/// first page's origin answers with. A linked document is returned even when
/// it does not parse, so the draft can say so; the probed one must parse.
pub(crate) async fn find(net: &Network, pages: &[Page]) -> Option<Description> {
    for page in pages {
        if let Some(link) = evidence::openapi_link(&page.html, &page.url) {
            return fetch(net, &link).await;
        }
    }
    let mut probe = url::Url::parse(&pages.first()?.url).ok()?;
    probe.set_query(None);
    probe.set_fragment(None);
    probe.set_path(DESCRIPTION_PATH);
    fetch(net, probe.as_str())
        .await
        .filter(|found| itda_prep::is_openapi(&found.bytes))
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
