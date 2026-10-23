//! `read`, `recon` and `search`: what the site shows, as evidence. A read
//! goes only to the site's registrable domain and to the hosts the site
//! itself linked as documentation, by GET, without an account.
use super::{PAGES, Session, coded, exhausted, feedback};
use crate::{Context, description, introspect, knowledge};
use itda_html::{docs, evidence, shape};
use itda_kb::{PREPARE_SCOPE, Ref};
use itda_net::Request;
use itda_prep::ground::{SeenPath, SeenRoute, SeenServer};
use itda_types::{
    error::{AppError, AppResult},
    url::registrable_domain,
};
use serde_json::{Value, json};

/// The most a page or document read may weigh.
const PAGE_LIMIT: usize = 4 * 1024 * 1024;
/// How much of a page is kept in memory for drafting.
const KEPT_HTML: usize = 2 * 1024 * 1024;
/// Headings at one level shown with their words; more say records.
const HEADINGS_SHOWN: usize = 5;
/// Rows of a page's structure and fields shown.
const STRUCTURE_SHOWN: usize = 40;
/// Routes taken from bundles that get one GET each.
const BUNDLE_PROBES: usize = 12;
/// Routes written to the knowledge base per read.
const KB_ROUTES: usize = 100;

/// A host that names this computer or a private network.
fn private_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// Whether `url` is one a read may go to: the site's own registrable
/// domain (its own origin when it has none, as an address or `localhost`
/// does), or a host the site linked as documentation. A private address
/// other than the site's own is refused whatever linked it.
pub(crate) fn scope(session: &Session, url: &url::Url) -> Result<(), AppError> {
    scope_of(&session.entry, &session.doc_hosts, url)
}

fn scope_of(
    entry: &url::Url,
    doc_hosts: &std::collections::BTreeSet<String>,
    url: &url::Url,
) -> Result<(), AppError> {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let entry_host = entry.host_str().unwrap_or_default().to_ascii_lowercase();
    let own = match (registrable_domain(&host), registrable_domain(&entry_host)) {
        (Some(a), Some(b)) => a == b,
        _ => url.origin() == entry.origin(),
    };
    if own {
        return Ok(());
    }
    if private_host(&host) {
        return Err(coded(
            "private_address",
            403,
            format!("{host} is a private address, not the site being prepared"),
            "Read pages of the site itself; itda reads no other address on this computer or network.",
        ));
    }
    if doc_hosts.contains(&host) {
        return Ok(());
    }
    Err(coded(
        "outside_site",
        403,
        format!("{host} is not {entry_host}'s site, nor a host it linked as documentation"),
        "Read pages on the site, or the API description and servers it links to.",
    ))
}

/// The address checked and in scope.
pub(crate) fn in_scope(session: &Session, raw: &str) -> AppResult<url::Url> {
    let url = itda_types::url::validate_url(raw)?;
    scope(session, &url)?;
    Ok(url)
}

/// Requests sent since `since`, by the turns the sites' pacing gave out:
/// direct, in the browser, redirects, all of them.
pub(crate) async fn turns(ctx: &Context, since: i64) -> usize {
    ctx.net.gate.turns_since(since).await.unwrap_or(1)
}

/// One GET as itda, anonymous, counted against the page budget.
async fn fetch(ctx: &Context, session: &mut Session, url: &str) -> AppResult<itda_net::Response> {
    if session.pages >= PAGES {
        return Err(exhausted("page reads"));
    }
    let since = itda_types::util::unix_ms();
    let result = ctx
        .net
        .fetch(Request {
            limit: PAGE_LIMIT,
            ..Request::read(url)
        })
        .await;
    session.pages += turns(ctx, since).await;
    result
}

/// Note something in the knowledge base under the preparation's scope. A
/// knowledge base that cannot be written does not stop the loop.
pub(crate) async fn note(ctx: &Context, session: &mut Session, node: Ref<'_>, statement: &str) {
    if let Err(error) = ctx
        .kb
        .finding(&session.site, node, statement, PREPARE_SCOPE, 1.0)
        .await
    {
        session.unrecorded = Some(error.msg);
    }
}

async fn node(ctx: &Context, session: &mut Session, kind: &str, key: &str, props: Value) {
    if let Err(error) = ctx.kb.node(&session.site, kind, key, props).await {
        session.unrecorded = Some(error.msg);
    }
}

fn origin_of(url: &url::Url) -> String {
    url.origin().ascii_serialization()
}

/// A route as one source names it.
struct Lead<'a> {
    method: &'a str,
    path: &'a str,
    origin: Option<String>,
    from: &'static str,
}

/// What was learned from a route, for the session and the knowledge base.
async fn route(
    ctx: &Context,
    session: &mut Session,
    lead: Lead<'_>,
    source: &str,
    written: &mut usize,
) -> String {
    let Lead {
        method,
        path,
        origin,
        from,
    } = lead;
    let id = session.add(
        "route",
        source,
        json!({"method": method, "path": path, "origin": origin, "from": from}),
        format!("{method} {path} {from}"),
    );
    session.seen.routes.push(SeenRoute {
        method: method.to_owned(),
        path: path.to_owned(),
        origin,
        evidence: id.clone(),
        unsupported_auth: false,
    });
    if *written < KB_ROUTES {
        *written += 1;
        let key = format!("{method} {path}");
        node(
            ctx,
            session,
            "endpoint",
            &key,
            json!({"from": from, "page": source}),
        )
        .await;
    }
    id
}

/// Paths as the agent sees them: a path with two or more siblings under
/// the same parent becomes the parent with `{}` and how many there are, so
/// the names of records a page links to (a user, an article) stay out.
fn path_patterns(paths: &[String]) -> Vec<Value> {
    let parent = |path: &str| {
        path.trim_end_matches('/')
            .rsplit_once('/')
            .map_or(String::new(), |(p, _)| p.to_owned())
    };
    let mut out: Vec<Value> = Vec::new();
    let mut folded: Vec<String> = Vec::new();
    for path in paths {
        let up = parent(path);
        let siblings = paths.iter().filter(|p| parent(p) == up).count();
        if siblings < 3 || up.is_empty() {
            out.push(json!(path));
        } else if !folded.contains(&up) {
            folded.push(up.clone());
            out.push(json!({"pattern": format!("{up}/{{}}"), "count": siblings}));
        }
    }
    out
}

/// The headings, with the words of the levels that are few enough to be a
/// page's own; a level with many headings is a list of records, counted.
fn headings_shown(html: &str) -> (Vec<Value>, Value) {
    let all = docs::headings(html);
    let count = |level: u8| all.iter().filter(|h| h.level == level).count();
    let counts = json!({"h1": count(1), "h2": count(2), "h3": count(3)});
    let shown = all
        .iter()
        .filter(|h| count(h.level) <= HEADINGS_SHOWN)
        .map(|h| json!({"level": h.level, "text": h.text}))
        .collect();
    (shown, counts)
}

/// Read one HTML page into evidence.
async fn html_evidence(
    ctx: &Context,
    session: &mut Session,
    url: &url::Url,
    html: &str,
    link: String,
) -> Value {
    let key = knowledge::page_key(url.as_str());
    let origin = origin_of(url);
    let title: String = itda_html::title(html).chars().take(120).collect();
    let (headings, heading_counts) = headings_shown(html);
    let page_shape = shape::page(html);
    let structure: Vec<Value> = page_shape["structure"]
        .as_array()
        .map(|rows| rows.iter().take(STRUCTURE_SHOWN).cloned().collect())
        .unwrap_or_default();
    let fields: Vec<Value> = page_shape["fields"]
        .as_array()
        .map(|rows| rows.iter().take(STRUCTURE_SHOWN).cloned().collect())
        .unwrap_or_default();
    let page_id = session.add(
        "page",
        &key,
        json!({"title": title}),
        format!("{title} {}", url.path()),
    );
    session.seen.servers.push(SeenServer {
        url: origin.clone(),
        evidence: page_id.clone(),
    });
    session.seen.paths.push(SeenPath {
        origin: origin.clone(),
        path: url.path().trim_end_matches('/').to_owned(),
        evidence: page_id.clone(),
        session: false,
    });
    let mut props = page_shape.clone();
    props["fetched"] = true.into();
    if props.get("truncated").is_none() {
        props["truncated"] = Value::Null;
    }
    node(ctx, session, "page", &key, props).await;

    let paths = docs::same_origin_paths(html, url.as_str());
    let links_id = (!paths.is_empty()).then(|| {
        // Searchable by the patterns, not by the names in them.
        let words: Vec<String> = path_patterns(&paths)
            .iter()
            .map(|p| match p.as_str() {
                Some(path) => path.to_owned(),
                None => p["pattern"].as_str().unwrap_or_default().to_owned(),
            })
            .collect();
        let id = session.add(
            "links",
            &key,
            json!({"paths": paths.len()}),
            words.join(" "),
        );
        for path in &paths {
            session.seen.paths.push(SeenPath {
                origin: origin.clone(),
                path: path.trim_end_matches('/').to_owned(),
                evidence: id.clone(),
                session: false,
            });
        }
        id
    });

    let mut written = 0;
    let mut routes = Vec::new();
    for found in docs::prose_routes(html) {
        let route_origin = found.origin.clone();
        if let Some(server) = &route_origin {
            // An absolute address in the site's own docs names a server.
            let id = session.add(
                "server",
                &key,
                json!({"url": server}),
                format!("server {server}"),
            );
            session.seen.servers.push(SeenServer {
                url: server.clone(),
                evidence: id,
            });
        }
        let lead = Lead {
            method: &found.method,
            path: &found.path,
            origin: route_origin,
            from: "docs",
        };
        let id = route(ctx, session, lead, &key, &mut written).await;
        routes.push(
            json!({"id": id, "method": found.method, "path": found.path, "origin": found.origin}),
        );
    }

    let mut forms = Vec::new();
    for form in evidence::forms(html, url.as_str()) {
        if form.password {
            continue;
        }
        let Ok(action) = url::Url::parse(&form.action) else {
            continue;
        };
        let method = form.method.to_ascii_uppercase();
        let controls: Vec<Value> = form
            .controls
            .iter()
            .filter(|c| !c.secret)
            .map(|c| json!({"name": c.name, "type": c.kind, "required": c.required, "hidden": c.hidden}))
            .collect();
        let action_origin = origin_of(&action);
        let lead = Lead {
            method: &method,
            path: action.path(),
            origin: Some(action_origin),
            from: "form",
        };
        let id = route(ctx, session, lead, &key, &mut written).await;
        node(
            ctx,
            session,
            "form",
            &format!("{method} {}", action.path()),
            json!({"page": key, "controls": controls.len()}),
        )
        .await;
        forms.push(
            json!({"id": id, "method": method, "action": action.path(), "controls": controls}),
        );
    }

    let mut lists = Vec::new();
    if let Some(extraction) = evidence::infer_extraction(html, url.as_str()) {
        let count = extraction
            .extract(html, url.as_str())
            .and_then(|v| v["items"].as_array().map(Vec::len))
            .unwrap_or(0);
        let names: Vec<&String> = extraction.fields.keys().collect();
        let id = session.add(
            "list",
            &key,
            json!({"items": extraction.items, "count": count}),
            format!(
                "list {} {}",
                extraction.items,
                names
                    .iter()
                    .map(|n| n.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
        let fields: serde_json::Map<String, Value> = extraction
            .fields
            .iter()
            .map(|(name, field)| {
                (
                    name.clone(),
                    json!({"selector": field.selector, "attribute": field.attribute}),
                )
            })
            .collect();
        lists.push(json!({"id": id, "items": extraction.items, "count": count, "fields": fields}));
    }

    let mut ssr = Vec::new();
    for (name, payload) in shape::ssr_payloads(html) {
        let id = session.add("ssr", &key, json!({"key": name}), format!("ssr {name}"));
        ssr.push(json!({"id": id, "key": name, "shape": payload}));
    }

    let mut feeds = Vec::new();
    for feed in evidence::feeds(html, url.as_str()) {
        if let Ok(target) = url::Url::parse(&feed.url) {
            let lead = Lead {
                method: "GET",
                path: target.path(),
                origin: Some(origin_of(&target)),
                from: "feed",
            };
            let id = route(ctx, session, lead, &key, &mut written).await;
            feeds.push(json!({"id": id, "path": target.path()}));
        }
    }

    let mut described = Value::Null;
    let mut linked = evidence::description_link(html, url.as_str())
        .map(|(rel, href)| (format!("{rel:?}"), href))
        .into_iter()
        .collect::<Vec<_>>();
    for (target, rels) in itda_types::url::link_header(&link) {
        if let (Some(rel), Ok(href)) = (
            evidence::DescriptionRel::of(rels.iter().map(String::as_str)),
            url.join(&target),
        ) {
            linked.push((format!("{rel:?}"), href.to_string()));
        }
    }
    if let Some((rel, href)) = linked.first().cloned()
        && let Ok(target) = url::Url::parse(&href)
    {
        let host = target.host_str().unwrap_or_default().to_ascii_lowercase();
        if !private_host(&host) || target.origin() == session.entry.origin() {
            session.doc_hosts.insert(host);
        }
        let shown_url = knowledge::page_key(&href);
        let id = session.add(
            "description_link",
            &key,
            json!({"rel": rel, "url": shown_url}),
            format!("description {shown_url}"),
        );
        described = json!({"id": id, "rel": rel, "url": shown_url});
    }

    let scripts = docs::script_sources(html, url.as_str()).len();
    let graphql = itda_prep::graphql::endpoints_in_page(&itda_prep::Page {
        url: url.to_string(),
        html: html.to_owned(),
        link: String::new(),
    })
    .len();
    note(
        ctx,
        session,
        Ref {
            kind: "page",
            key: &key,
        },
        &format!(
            "prepare: read {page_id}: {} routes, {} forms, {} lists, {} embedded payloads, {} scripts",
            routes.len(),
            forms.len(),
            lists.len(),
            ssr.len(),
            scripts
        ),
    )
    .await;

    let mut kept = html.to_owned();
    if kept.len() > KEPT_HTML {
        let mut end = KEPT_HTML;
        while !kept.is_char_boundary(end) {
            end -= 1;
        }
        kept.truncate(end);
    }
    if !session.kept_pages.iter().any(|p| p.url == url.as_str()) {
        session.kept_pages.push(itda_prep::Page {
            url: url.to_string(),
            html: kept,
            link,
        });
    }
    json!({
        "kind": "html",
        "id": page_id,
        "page": key,
        "title": title,
        "headings": headings,
        "heading_counts": heading_counts,
        "links": {"id": links_id, "paths": path_patterns(&paths)},
        "routes": routes,
        "forms": forms,
        "lists": lists,
        "ssr": ssr,
        "feeds": feeds,
        "description": described,
        "structure": structure,
        "fields": fields,
        "scripts": scripts,
        "graphql_endpoints": graphql,
    })
}

/// An API description into evidence: its servers, which the site thereby
/// links as documentation hosts, and its routes under each server.
async fn description_evidence(
    ctx: &Context,
    session: &mut Session,
    url: &url::Url,
    summary: itda_prep::ApiSummary,
) -> Value {
    let key = knowledge::page_key(url.as_str());
    let id = session.add(
        "description",
        &key,
        json!({"title": summary.title, "servers": summary.servers.len(), "routes": summary.routes.len()}),
        format!("openapi description {}", summary.title),
    );
    node(
        ctx,
        session,
        "description",
        &key,
        json!({"servers": summary.servers.len(), "routes": summary.routes.len()}),
    )
    .await;
    let mut servers = Vec::new();
    let mut bases: Vec<(String, String)> = Vec::new();
    for server in &summary.servers {
        let Ok(parsed) = url::Url::parse(server) else {
            continue;
        };
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        if !private_host(&host) || parsed.origin() == session.entry.origin() {
            session.doc_hosts.insert(host);
        }
        let server_id = session.add(
            "server",
            &key,
            json!({"url": knowledge::page_key(server)}),
            format!("server {server}"),
        );
        session.seen.servers.push(SeenServer {
            url: server.clone(),
            evidence: server_id.clone(),
        });
        servers.push(json!({"id": server_id, "url": knowledge::page_key(server)}));
        bases.push((
            origin_of(&parsed),
            parsed.path().trim_end_matches('/').to_owned(),
        ));
    }
    if bases.is_empty() {
        bases.push((origin_of(url), String::new()));
    }
    let mut routes = Vec::new();
    let mut written = 0;
    for found in &summary.routes {
        let route_id = session.add(
            "route",
            &key,
            json!({"method": found.method, "path": found.path, "from": "openapi", "unsupported_auth": found.unsupported_auth}),
            format!("{} {} openapi", found.method, found.path),
        );
        for (origin, prefix) in &bases {
            session.seen.routes.push(SeenRoute {
                method: found.method.clone(),
                path: format!("{prefix}{}", found.path),
                origin: Some(origin.clone()),
                evidence: route_id.clone(),
                unsupported_auth: found.unsupported_auth,
            });
        }
        if written < KB_ROUTES {
            written += 1;
            node(
                ctx,
                session,
                "endpoint",
                &format!("{} {}", found.method, found.path),
                json!({"from": "openapi", "page": key, "unsupported_auth": found.unsupported_auth}),
            )
            .await;
        }
        routes.push(json!({"id": route_id, "method": found.method, "path": found.path, "unsupported_auth": found.unsupported_auth}));
    }
    note(
        ctx,
        session,
        Ref {
            kind: "description",
            key: &key,
        },
        &format!(
            "prepare: description {id}: {} servers, {} routes",
            servers.len(),
            routes.len()
        ),
    )
    .await;
    json!({"kind": "openapi", "id": id, "page": key, "title": summary.title, "servers": servers, "routes": routes})
}

/// A JSON answer, or a document that is neither HTML nor a description,
/// into evidence: its shape, never its values.
async fn document_evidence(
    ctx: &Context,
    session: &mut Session,
    url: &url::Url,
    response: &itda_net::Response,
) -> Value {
    let key = knowledge::page_key(url.as_str());
    if let Some(summary) = itda_prep::api_summary(&response.bytes, Some(url.as_str())) {
        if session.description.is_none() {
            session.description = Some(itda_prep::Description {
                url: url.to_string(),
                bytes: response.bytes.clone(),
            });
        }
        return description_evidence(ctx, session, url, summary).await;
    }
    let content_type = response
        .header("content-type")
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let Some(json) = response.json() else {
        let id = session.add(
            "document",
            &key,
            json!({"content_type": content_type, "bytes": response.bytes.len()}),
            format!("document {content_type}"),
        );
        return json!({"kind": "document", "id": id, "page": key, "content_type": content_type, "bytes": response.bytes.len()});
    };
    let shaped = shape::shape(&json);
    let pointers: Vec<Value> = shape::pointers(&shaped)
        .into_iter()
        .take(60)
        .map(|p| json!({"pointer": p.pointer, "type": p.kind}))
        .collect();
    let answered = itda_prep::ground::answered(&json);
    let id = session.add(
        "json",
        &key,
        json!({"answered": answered}),
        format!(
            "json {} {}",
            url.path(),
            pointers
                .iter()
                .filter_map(|p| p["pointer"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    if answered {
        let origin = origin_of(url);
        session.seen.servers.push(SeenServer {
            url: origin.clone(),
            evidence: id.clone(),
        });
        session.seen.paths.push(SeenPath {
            origin,
            path: url.path().trim_end_matches('/').to_owned(),
            evidence: id.clone(),
            session: false,
        });
    }
    node(
        ctx,
        session,
        "endpoint",
        &format!("GET {}", url.path()),
        json!({"from": "read", "shape": shaped}),
    )
    .await;
    json!({"kind": "json", "id": id, "page": key, "answered": answered, "shape": shaped, "pointers": pointers})
}

/// What a refused read says, and that nothing more goes to the site.
fn refused_reply(session: &mut Session, url: &url::Url, code: &str, why: String) -> Value {
    let host = url.host_str().unwrap_or_default().to_owned();
    let blocked = matches!(
        code,
        "blocked_by_site" | "site_rate_limited" | "auth_required"
    );
    if blocked {
        session.refused = true;
    }
    let advice = if code == "auth_required" {
        "the page needs a sign-in; pause and tell the user, who can sign in with itda_login once the site is installed"
    } else if blocked {
        "the site refused itda; pause and tell the user what it said"
    } else {
        "the page did not answer as a page; read another the site links to"
    };
    json!({
        "status": code,
        "page": knowledge::page_key(url.as_str()),
        "message": why,
        "feedback": [feedback(if blocked { "blocked" } else { "unreadable" }, host, advice, Value::Null)],
    })
}

/// `read`: one page or document on the site, once.
pub(crate) async fn read(ctx: &Context, session: &mut Session, raw: &str) -> AppResult<Value> {
    let url = in_scope(session, raw)?;
    let key = format!("read|{}", url.as_str());
    if let Some(previous) = session.done.get(&key) {
        return Ok(json!({
            "status": "ok",
            "already": true,
            "result": previous,
            "feedback": [feedback("already", knowledge::page_key(url.as_str()), "this page was read in this session; its evidence ids still hold", Value::Null)],
        }));
    }
    if session.refused {
        return Ok(refused_reply(
            session,
            &url,
            "blocked_by_site",
            "the site refused an earlier request in this session".into(),
        ));
    }
    let result = read_url(ctx, session, &url).await?;
    if result["status"] == "ok" {
        session.done.insert(key, result.clone());
    }
    let mut reply = result;
    reply["budgets"] = session.budgets();
    Ok(reply)
}

/// Fetch `url` and turn what it answered into evidence.
async fn read_url(ctx: &Context, session: &mut Session, url: &url::Url) -> AppResult<Value> {
    let response = match fetch(ctx, session, url.as_str()).await {
        Ok(response) => response,
        Err(error) if error.public_code == "budget_exhausted" => return Err(error),
        Err(error) => {
            let message = error.msg.clone();
            return Ok(refused_reply(session, url, &error.public_code, message));
        }
    };
    if let Some(refused) = crate::refusal(url.as_str(), &response) {
        let message = refused.error.msg.clone();
        return Ok(refused_reply(
            session,
            url,
            &refused.error.public_code,
            message,
        ));
    }
    if !response.successful() {
        return Ok(refused_reply(
            session,
            url,
            "request_failed",
            format!("the site answered HTTP {}", response.status),
        ));
    }
    // A redirect stays on the origin it started from.
    let landed = url::Url::parse(&response.url).unwrap_or_else(|_| url.clone());
    let content_type = response.header("content-type").to_ascii_lowercase();
    let mut shown = if content_type.contains("html") {
        let html = String::from_utf8_lossy(&response.bytes).into_owned();
        let link = response.headers_joined("link");
        html_evidence(ctx, session, &landed, &html, link).await
    } else {
        document_evidence(ctx, session, &landed, &response).await
    };
    shown["status"] = "ok".into();
    shown["http_status"] = response.status.into();
    Ok(shown)
}

/// The query fields a GraphQL schema offers, by name.
fn query_fields(schema: &Value) -> Vec<String> {
    let root = &schema["data"]["__schema"];
    let Some(query) = root["queryType"]["name"].as_str() else {
        return Vec::new();
    };
    root["types"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["name"].as_str() == Some(query))
        .and_then(|t| t["fields"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|f| f["name"].as_str().map(str::to_owned))
        .take(40)
        .collect()
}

/// An operation of a draft as the agent sees it: where it goes, what it
/// takes, what it checks; never a default value.
fn candidate(spec: &itda_spec::Spec, op: &itda_spec::Operation) -> Value {
    let inputs: Vec<Value> = op
        .params
        .iter()
        .chain(op.query.iter())
        .chain(op.body.iter().flat_map(|b| b.inputs()))
        .filter(|(_, p)| !p.from_step())
        .map(|(name, p)| json!({"name": name, "type": p.kind, "required": p.required || op.params.contains_key(name), "has_default": p.default.is_some()}))
        .collect();
    let check = op.response.as_ref().map(|r| {
        json!({
            "format": r.format,
            "required_pointers": r.required_pointers,
            "extract_pointer": r.extract_pointer,
            "ssr_json": r.ssr_json,
            "html_items": r.html.as_ref().map(|h| h.items.clone()),
            "html_fields": r.required_html_fields.len(),
        })
    });
    json!({
        "name": op.name,
        "desc": op.desc,
        "method": if op.method.is_empty() { "GET" } else { op.method.as_str() },
        "path": op.path,
        "base_url": spec.operation_base(op),
        "inputs": inputs,
        "response": check,
    })
}

/// `recon`: the entry page, the API description, GraphQL, the routes the
/// docs name and the draft itda makes from all of it, in one call;
/// `bundles` also reads the scripts the entry page loads for route
/// strings, keeping only those that answer one GET with data.
pub(crate) async fn recon(ctx: &Context, session: &mut Session, bundles: bool) -> AppResult<Value> {
    let key = format!("recon|{bundles}");
    if let Some(previous) = session.done.get(&key) {
        return Ok(json!({
            "status": "ok",
            "already": true,
            "result": previous,
            "feedback": [feedback("already", &session.site, "recon ran in this session with the same arguments", json!({"op": "propose", "session": session.id}))],
        }));
    }
    let mut notes = Vec::new();
    let entry = session.entry.clone();
    let mut entry_page = Value::Null;
    if !session.kept_pages.iter().any(|p| p.url == entry.as_str()) && !session.refused {
        let result = read_url(ctx, session, &entry).await?;
        if result["status"] == "ok" {
            session
                .done
                .insert(format!("read|{}", entry.as_str()), result.clone());
        }
        entry_page = result;
    }
    if session.refused {
        let mut reply = if entry_page.is_null() {
            json!({
                "status": "blocked_by_site",
                "message": "the site refused an earlier request in this session",
                "feedback": [feedback("blocked", &session.site, "the site refused itda; pause and tell the user what it said", Value::Null)],
            })
        } else {
            entry_page
        };
        reply["budgets"] = session.budgets();
        return Ok(reply);
    }
    let pages = session.kept_pages.clone();
    // The description the pages link to, or `/openapi.json`, within scope.
    let mut described = Value::Null;
    if session.description.is_none() && !pages.is_empty() {
        let scope_of = SessionScope::of(session);
        let since = itda_types::util::unix_ms();
        let found = description::find_within(&ctx.net, &pages, &|url| scope_of.allows(url)).await;
        session.pages += turns(ctx, since).await;
        if let Some(found) = found
            && let Ok(url) = url::Url::parse(&found.url)
            && let Some(summary) = itda_prep::api_summary(&found.bytes, Some(&found.url))
        {
            session.description = Some(found);
            described = description_evidence(ctx, session, &url, summary).await;
        }
    }
    // What the GraphQL endpoints the pages name offer.
    let mut graphql = Vec::new();
    if session.schemas.is_empty() {
        let endpoints: Vec<String> = introspect::endpoints_in_pages(&pages)
            .into_iter()
            .filter(|e| in_scope(session, e).is_ok())
            .collect();
        if !endpoints.is_empty() && session.pages < PAGES {
            let since = itda_types::util::unix_ms();
            session.schemas = introspect::schemas(&ctx.net, &endpoints).await;
            session.pages += turns(ctx, since).await;
        }
        for schema in session.schemas.clone() {
            let Ok(url) = url::Url::parse(&schema.url) else {
                continue;
            };
            let fields = query_fields(&schema.json);
            let page = knowledge::page_key(&schema.url);
            let id = session.add(
                "graphql",
                &page,
                json!({"fields": fields.len()}),
                format!("graphql {}", fields.join(" ")),
            );
            session.seen.servers.push(SeenServer {
                url: origin_of(&url),
                evidence: id.clone(),
            });
            session.seen.routes.push(SeenRoute {
                method: "POST".into(),
                path: url.path().to_owned(),
                origin: Some(origin_of(&url)),
                evidence: id.clone(),
                unsupported_auth: false,
            });
            for field in &fields {
                node(
                    ctx,
                    session,
                    "graphql_field",
                    &format!("{page}#{field}"),
                    json!({"endpoint": page}),
                )
                .await;
            }
            graphql.push(json!({"id": id, "endpoint": page, "query_fields": fields}));
        }
    }
    // The draft itda makes from the same evidence, judged like the agent's.
    let mut candidates = Vec::new();
    if !pages.is_empty() {
        let options = itda_prep::Options {
            name: Some(session.site.clone()),
            lang: ctx.lang.clone(),
            schemas: session.schemas.clone(),
            ..itda_prep::Options::default()
        };
        match itda_prep::from_pages_and_description(&pages, session.description.as_ref(), &options)
            .and_then(|draft| itda_spec::parse(draft.yaml.as_bytes()))
        {
            Ok(spec) => {
                let grounded = itda_prep::ground::ground(&spec, &session.seen);
                for op in &spec.operations {
                    let mut shown = candidate(&spec, op);
                    if let Some(j) = grounded.judgments.iter().find(|j| j.op == op.name) {
                        shown["verdict"] = j.verdict.into();
                        shown["effect"] = j.effect.into();
                        shown["reason"] = j.reason.clone().into();
                        shown["evidence_ids"] = json!(j.evidence_ids);
                    }
                    candidates.push(shown);
                }
                session.recon = Some(spec);
            }
            Err(error) => notes.push(feedback(
                "no_draft",
                &session.site,
                format!("itda drafted nothing from these pages: {}", error.msg),
                json!({"op": "read", "session": session.id}),
            )),
        }
    }
    let bundled = if bundles {
        Some(mine_bundles(ctx, session).await?)
    } else {
        None
    };
    let routes: Vec<Value> = session
        .evidence
        .iter()
        .filter(|e| e.kind == "route")
        .take(80)
        .map(|e| {
            let mut shown = e.shown.clone();
            shown["id"] = e.id.clone().into();
            shown
        })
        .collect();
    notes.push(feedback(
        "next",
        &session.site,
        "draft the actions in plain words from this evidence; candidates itda kept can be taken whole with use_recon",
        json!({"op": "propose", "session": session.id, "use_recon": true}),
    ));
    let reply = json!({
        "status": "ok",
        "entry": entry_page,
        "description": described,
        "graphql": graphql,
        "routes": routes,
        "candidates": candidates,
        "bundles": bundled,
        "budgets": session.budgets(),
        "feedback": notes,
    });
    session.done.insert(key, reply.clone());
    Ok(reply)
}

/// What a read may reach, copied out of a session for a filter that
/// outlives the borrow.
struct SessionScope {
    entry: url::Url,
    doc_hosts: std::collections::BTreeSet<String>,
}

impl SessionScope {
    fn of(session: &Session) -> Self {
        Self {
            entry: session.entry.clone(),
            doc_hosts: session.doc_hosts.clone(),
        }
    }

    fn allows(&self, raw: &str) -> bool {
        let Ok(url) = itda_types::url::validate_url(raw) else {
            return false;
        };
        scope_of(&self.entry, &self.doc_hosts, &url).is_ok()
    }
}

/// Route strings in the scripts the entry page loads, each GET tried once:
/// only those that answer with data become evidence; the rest are counted
/// as noise.
async fn mine_bundles(ctx: &Context, session: &mut Session) -> AppResult<Value> {
    let entry = session.entry.clone();
    // The entry page, or the page it redirected to: the first one read.
    let Some(page) = session
        .kept_pages
        .iter()
        .find(|p| p.url == entry.as_str())
        .or_else(|| session.kept_pages.first())
        .cloned()
    else {
        return Ok(json!({"scripts": 0}));
    };
    let scripts: Vec<String> = docs::script_sources(&page.html, &page.url)
        .into_iter()
        .filter(|s| in_scope(session, s).is_ok())
        .collect();
    let mut read_scripts = 0;
    let mut leads: Vec<docs::Route> = Vec::new();
    for script in &scripts {
        if session.pages >= PAGES || session.refused {
            break;
        }
        let Ok(response) = fetch(ctx, session, script).await else {
            continue;
        };
        if !response.successful() {
            continue;
        }
        read_scripts += 1;
        for found in docs::bundle_routes(&String::from_utf8_lossy(&response.bytes)) {
            if !leads.contains(&found) {
                leads.push(found);
            }
        }
    }
    let documented = |method: &str, path: &str| {
        session
            .seen
            .routes
            .iter()
            .any(|r| r.method == method && itda_prep::ground::route_matches(&r.path, path))
    };
    let probes: Vec<docs::Route> = leads
        .iter()
        .filter(|r| r.method == "GET" && !r.path.contains('{') && !documented(&r.method, &r.path))
        .take(BUNDLE_PROBES)
        .cloned()
        .collect();
    let (mut answered, mut noise) = (Vec::new(), 0usize);
    let origin = origin_of(&entry);
    for lead in &probes {
        if session.pages >= PAGES || session.refused {
            break;
        }
        let target = format!("{origin}{}", lead.path);
        let Ok(response) = fetch(ctx, session, &target).await else {
            noise += 1;
            continue;
        };
        let data = response
            .successful()
            .then(|| response.json())
            .flatten()
            .filter(itda_prep::ground::answered);
        let Some(data) = data else {
            noise += 1;
            continue;
        };
        let id = session.add(
            "bundle_route",
            &knowledge::page_key(&target),
            json!({"method": "GET", "path": lead.path, "shape": shape::shape(&data)}),
            format!("GET {} bundle", lead.path),
        );
        session.seen.routes.push(SeenRoute {
            method: "GET".into(),
            path: lead.path.clone(),
            origin: Some(origin.clone()),
            evidence: id.clone(),
            unsupported_auth: false,
        });
        node(
            ctx,
            session,
            "endpoint",
            &format!("GET {}", lead.path),
            json!({"from": "bundle", "shape": shape::shape(&data)}),
        )
        .await;
        answered.push(json!({"id": id, "method": "GET", "path": lead.path}));
    }
    note(
        ctx,
        session,
        Ref {
            kind: "page",
            key: &knowledge::page_key(entry.as_str()),
        },
        &format!(
            "prepare: bundles: {read_scripts} scripts, {} route strings, {} tried, {} answered, {noise} noise",
            leads.len(),
            probes.len(),
            answered.len()
        ),
    )
    .await;
    Ok(json!({
        "scripts": scripts.len(),
        "scripts_read": read_scripts,
        "route_strings": leads.len(),
        "tried": probes.len(),
        "answered": answered,
        "noise": noise,
        "not_tried": leads.len() - probes.len(),
    }))
}

/// `search`: this session's evidence whose words hold every term, and the
/// knowledge base's notes about the site that match.
pub(crate) async fn search(ctx: &Context, session: &Session, q: &str) -> AppResult<Value> {
    let terms: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return Err(itda_types::error::usage(
            "q is empty",
            "Pass words to look for, such as a path or a field name.",
        ));
    }
    let evidence: Vec<Value> = session
        .evidence
        .iter()
        .filter(|e| terms.iter().all(|t| e.words.contains(t.as_str())))
        .take(30)
        .map(|e| json!({"id": e.id, "kind": e.kind, "source": e.source, "shown": e.shown}))
        .collect();
    let findings: Vec<Value> = ctx
        .kb
        .search(Some(&session.site), q)
        .await
        .unwrap_or_default()
        .into_iter()
        .take(20)
        .map(|f| json!({"statement": f.statement, "scope": f.scope}))
        .collect();
    Ok(json!({
        "status": "ok",
        "evidence": evidence,
        "knowledge": findings,
        "budgets": session.budgets(),
        "feedback": [],
    }))
}
