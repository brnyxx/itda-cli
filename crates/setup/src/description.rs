//! The API description a site publishes, for `itda draft page` and `itda
//! setup`: the one the first page's `Link` header or a page's links point at,
//! or the one `/openapi.json` answers with. Fetched as itda, by GET, under a
//! size cap; whatever is not there is simply not there, so nothing here fails
//! the page draft.
use itda_html::evidence::{self, DescriptionRel};
use itda_net::{Network, Request};
use itda_prep::{DESCRIPTION_PATH, Description, Page};
use serde_json::Value;

/// Descriptions are read whole; huge ones are better drafted on their own.
const LIMIT: usize = 4 * 1024 * 1024;

/// The descriptions of an API catalog tried, at most.
const CATALOG_TRIES: usize = 3;

/// The description the site points at, best first: `service-desc` (RFC
/// 8631), then an `api-catalog` (RFC 9727) and its `service-desc` links, then
/// `describedby`; the first page's `Link` header before the pages' own links.
/// Learning where costs no request. Else the one `/openapi.json` on the
/// first page's origin answers with. A `service-desc` document is returned
/// even when it does not parse, so the draft can say so; anything else must
/// parse.
pub async fn find(net: &Network, pages: &[Page]) -> Option<Description> {
    find_within(net, pages, &|_| true).await
}

/// The same, fetching only addresses `allowed` accepts: a link elsewhere
/// is not followed.
pub async fn find_within(
    net: &Network,
    pages: &[Page],
    allowed: &(dyn Fn(&str) -> bool + Sync),
) -> Option<Description> {
    let first = pages.first()?;
    let mut linked = header_links(first);
    linked.extend(
        pages
            .iter()
            .filter_map(|page| evidence::description_link(&page.html, &page.url)),
    );
    if let Some((kind, link)) = linked.into_iter().min_by_key(|(kind, _)| *kind) {
        match kind {
            DescriptionRel::ServiceDesc => return fetch(net, &link, allowed).await,
            DescriptionRel::ApiCatalog => return catalog(net, &link, allowed).await,
            DescriptionRel::DescribedBy => {
                if let Some(found) = described(net, &link, allowed).await {
                    return Some(found);
                }
            }
        }
    }
    let mut probe = url::Url::parse(&first.url).ok()?;
    probe.set_query(None);
    probe.set_fragment(None);
    probe.set_path(DESCRIPTION_PATH);
    described(net, probe.as_str(), allowed).await
}

/// The description links of the page's `Link` header, resolved against the
/// page's address (RFC 8288, section 3.1).
fn header_links(page: &Page) -> Vec<(DescriptionRel, String)> {
    let Ok(base) = url::Url::parse(&page.url) else {
        return Vec::new();
    };
    itda_types::url::link_header(&page.link)
        .into_iter()
        .filter_map(|(target, rels)| {
            let kind = DescriptionRel::of(rels.iter().map(String::as_str))?;
            Some((kind, base.join(&target).ok()?.to_string()))
        })
        .collect()
}

/// An API catalog is a linkset (RFC 9264): `{"linkset": [{"service-desc":
/// [{"href": ...}]}]}`. Its `service-desc` links are tried in order, up to
/// [`CATALOG_TRIES`], and the first that parses is the description. A
/// catalog that links to further catalogs is not followed.
async fn catalog(
    net: &Network,
    url: &str,
    allowed: &(dyn Fn(&str) -> bool + Sync),
) -> Option<Description> {
    let listed = fetch(net, url, allowed).await?;
    let base = url::Url::parse(&listed.url).ok()?;
    let linkset: Value = serde_json::from_slice(&listed.bytes).ok()?;
    let links: Vec<String> = linkset["linkset"]
        .as_array()?
        .iter()
        .filter_map(|context| context["service-desc"].as_array())
        .flatten()
        .filter_map(|target| target["href"].as_str())
        .filter_map(|href| base.join(href).ok())
        .map(|href| href.to_string())
        .take(CATALOG_TRIES)
        .collect();
    for link in links {
        if let Some(found) = described(net, &link, allowed).await {
            return Some(found);
        }
    }
    None
}

/// The document at `url` when it is an OpenAPI or Swagger description.
async fn described(
    net: &Network,
    url: &str,
    allowed: &(dyn Fn(&str) -> bool + Sync),
) -> Option<Description> {
    fetch(net, url, allowed)
        .await
        .filter(|found| itda_prep::is_openapi(&found.bytes))
}

async fn fetch(
    net: &Network,
    url: &str,
    allowed: &(dyn Fn(&str) -> bool + Sync),
) -> Option<Description> {
    if !allowed(url) {
        return None;
    }
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
