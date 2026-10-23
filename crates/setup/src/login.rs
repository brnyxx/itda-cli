//! Sign-in: the person signs in to a site in their own browser, or types
//! their user name and password for itda to send through the site's own
//! sign-in form, and itda keeps that site's session cookies. Nothing here
//! reads a terminal; the caller asks the person.
use crate::Context;
use itda_net::{Network, Request, Response};
use itda_spec::{LoginForm, Spec};
use itda_types::{
    error::{AppError, AppResult, usage},
    url::{same_site_origin, validate_url},
};
use itda_vault::Secret;
use std::{sync::mpsc, time::Duration};

/// A signed-in session as a `Cookie` header value, how many cookies it
/// holds, and, when the site carries its session in the address instead,
/// the query key and value the sign-in handed out.
pub struct Session {
    pub cookies: String,
    pub count: usize,
    pub query: Option<(String, String)>,
    /// The bearer token the signed-in page kept in browser storage, with the
    /// key it was under, when the site keeps its session there rather than in
    /// a cookie.
    pub storage: Option<(String, String)>,
}

impl Session {
    /// What goes in the keyring for a definition whose `value_from` is
    /// `from`: the storage token for `storage:<key>`, the query value for
    /// `query:<key>`, else the cookies.
    pub fn secret_for(&self, from: &str) -> Secret {
        if let Some((key, value)) = &self.storage
            && from.strip_prefix("storage:") == Some(key.as_str())
        {
            return Secret::new(value.clone());
        }
        match (&self.query, from.strip_prefix("query:")) {
            (Some((key, value)), Some(named)) if key == named => Secret::new(value.clone()),
            _ => Secret::new(self.cookies.clone()),
        }
    }

    /// The `value_from` a definition should carry for this session: the
    /// query key when the sign-in handed one out, else the cookie, else none.
    pub fn value_from(&self, cookie: Option<&str>) -> String {
        match (&self.query, cookie) {
            (Some((key, _)), _) => format!("query:{key}"),
            (None, Some(cookie)) => format!("cookie:{cookie}"),
            (None, None) => String::new(),
        }
    }
}

/// What the sign-in says while it waits.
pub enum Progress {
    /// The window is open: sign in there.
    Opened { site: String },
    /// The sign-in ends when this cookie (or storage key) appears, or when
    /// the person says so.
    WaitsFor { cookie: Option<String> },
}

/// The `value_from` of the entry a browser sign-in fills: a `browser`
/// credential (cookies, or a query key the form hands out), or a `header`
/// credential whose value the site keeps in browser storage (`storage:<key>`).
pub fn session_value_from(spec: &Spec) -> Option<&str> {
    spec.auth
        .iter()
        .find(|auth| {
            auth.kind == "browser"
                || (auth.kind == "header" && auth.value_from.starts_with("storage:"))
        })
        .map(|auth| auth.value_from.as_str())
}

/// Open the browser at the site's sign-in page and wait until the person is
/// signed in: the sign-in cookie the definition names appears, the bearer
/// token it keeps in storage appears, or `done` fires. Without `done` (no
/// terminal to press Enter in), the definition must name the cookie or the
/// storage key, or there is nothing to wait for. Blocks; run it off the async
/// runtime.
pub fn browser(
    spec: &Spec,
    page: Option<&str>,
    done: Option<&mpsc::Receiver<()>>,
    report: &mut dyn FnMut(Progress),
) -> AppResult<Session> {
    let auth = spec
        .auth
        .iter()
        .find(|auth| {
            auth.kind == "browser"
                || (auth.kind == "header" && auth.value_from.starts_with("storage:"))
        })
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
    let storage_key = auth.value_from.strip_prefix("storage:");
    let required = auth.value_from.strip_prefix("cookie:");
    if done.is_none() && required.is_none() && storage_key.is_none() {
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
        cookie: required.or(storage_key).map(str::to_owned),
    });
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if !window.is_open() {
            return Err(usage(
                "The browser was closed before sign-in finished",
                "Run the command again and leave the window open until itda continues.",
            ));
        }
        // A session the site keeps in the page's own storage: wait for the
        // named key to hold a token, then keep that value.
        if let Some(key) = storage_key {
            let token = window
                .storage(&scope)?
                .into_iter()
                .find(|item| item.key == key)
                .map(|item| item.value);
            let pressed = done.is_some_and(|done| done.try_recv().is_ok());
            if let Some(value) = token.filter(|value| !value.trim().is_empty()) {
                return Ok(Session {
                    cookies: String::new(),
                    count: 0,
                    query: None,
                    storage: Some((key.to_owned(), value)),
                });
            }
            if pressed {
                return Err(usage(
                    format!("The browser has no {key} value in storage for this site"),
                    "Finish signing in before pressing Enter.",
                ));
            }
            continue;
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
                query: None,
                storage: None,
            });
        }
    }
}

/// Keep the session as `site/account` and lift any pause on the site: a new
/// sign-in replaces an expired one, so the account may run again. `from` is
/// the definition's `value_from`, which says whether the cookies or the
/// query value are the session.
pub async fn keep(
    ctx: &Context,
    site: &str,
    account: &str,
    label: &str,
    session: Session,
    from: &str,
) -> AppResult<()> {
    ctx.vault.add_account(site, account, label).await?;
    ctx.vault
        .set_secret(
            site,
            account,
            itda_runtime::SESSION_SECRET,
            session.secret_for(from),
        )
        .await?;
    // Approvals were bound to the credential the user saw; a new one needs
    // a new look.
    ctx.approvals.invalidate(Some(site), Some(account)).await?;
    crate::resume(ctx, site, Some(account)).await
}

/// What the person typed for the site's own sign-in form.
pub struct Credentials {
    pub user: String,
    pub password: Secret,
}

/// A sign-in through the site's own form: the session (its cookies, or the
/// query value the site handed out instead), the cookie that appeared with
/// it (the one a definition can name as proof of the sign-in), and the form
/// as a definition describes it, so `itda login` can sign in again the same
/// way.
pub struct Signed {
    pub session: Session,
    pub cookie: Option<String>,
    pub form: LoginForm,
}

/// Sign in through the site's own form, as itda: read the page at `page`
/// (on `scope`'s site), fill the user and password fields of its sign-in
/// form, send it once with the hidden fields the page gave, and keep the
/// cookies the site answered with. `fields` names the user and password
/// fields when a definition already knows them. A wrong password is
/// reported, never retried; the password goes to the form's own action and
/// nowhere else, and nothing is paused on the site.
pub async fn form(
    net: &Network,
    scope: &url::Url,
    page: &str,
    fields: Option<(&str, &str)>,
    credentials: &Credentials,
    interval: Option<Duration>,
) -> AppResult<Signed> {
    let page = validate_url(page)?;
    if !same_site_origin(scope, &page) {
        return Err(usage(
            "The sign-in page is on another site",
            format!("Use a page on {}.", scope.origin().ascii_serialization()),
        ));
    }
    let host = page.host_str().unwrap_or_default().to_owned();
    let mut request = Request::read(page.as_str());
    request.interval = interval;
    let form_page = net.fetch(request).await?;
    if !form_page.successful() {
        return Err(itda_runtime::refusal(&form_page, false));
    }
    let html = String::from_utf8_lossy(&form_page.bytes);
    let (user_field, password_field) = fields.map_or((None, None), |(u, p)| (Some(u), Some(p)));
    let form = itda_html::evidence::sign_in_form(&html, &form_page.url, user_field, password_field)
        .ok_or_else(|| {
            usage(
                format!("itda found no sign-in form on {}", form_page.url),
                "Give the address of the page with the user name and password fields.",
            )
        })?;
    let action = validate_url(&form.action)?;
    if !same_site_origin(scope, &action) {
        return Err(usage(
            format!("The sign-in form on {host} sends the password to another site"),
            format!(
                "itda only signs in on {}; use the site's own sign-in page.",
                scope.origin().ascii_serialization()
            ),
        ));
    }
    let body = {
        let mut body = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in &form.fields {
            body.append_pair(name, value);
        }
        body.append_pair(&form.user, &credentials.user);
        body.append_pair(&form.password, credentials.password.expose());
        body.finish()
    };
    let before = set_cookies(&form_page);

    let mut request = Request::read(action.as_str());
    request.method = "POST".into();
    request.read_only = false;
    request.follow_redirects = false;
    request.interval = interval;
    request.body = Some(body.into_bytes());
    request.headers.push((
        "content-type".into(),
        "application/x-www-form-urlencoded".into(),
    ));
    if !before.is_empty() {
        request.headers.push(("cookie".into(), header(&before)));
    }
    let answer = match net.fetch(request).await {
        Ok(answer) => answer,
        Err(error) if error.public_code == "timeout" || error.public_code == "request_failed" => {
            let mut error = error;
            error.remedy = "Check the site and your connection, then run the command again.".into();
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    if answer.defense != "none" {
        return Err(itda_runtime::refusal(&answer, false));
    }
    if !matches!(answer.status, 301 | 302 | 303 | 307 | 308) {
        let mut error = AppError::api("auth_required", 401);
        error.msg = if answer.status == 200 {
            format!("{host} did not accept the user name or password")
        } else {
            format!("{host} answered the sign-in with HTTP {}", answer.status)
        };
        error.remedy = "Check them and run the command again.".into();
        return Err(error);
    }
    let after = set_cookies(&answer);
    let jar = merge(before, &after);
    // A site that keeps its session in the address hands it out with the
    // redirect, or on the links of the first page after the sign-in.
    let mut query = session_in_redirect(&action, &answer);
    if query.is_none() && jar.is_empty() {
        query = session_on_next_page(net, scope, &action, &answer, interval).await;
    }
    if jar.is_empty() && query.is_none() {
        return Err(usage(
            format!("{host} accepted the sign-in but set no cookie"),
            "The site keeps its session some other way; sign in with --browser instead.",
        ));
    }
    let cookie = proof(&after);
    let form = LoginForm {
        path: match page.query() {
            Some(query) => format!("{}?{query}", page.path()),
            None => page.path().to_owned(),
        },
        user: form.user,
        password: form.password,
    };
    Ok(Signed {
        session: Session {
            count: jar.len(),
            cookies: header(&jar),
            query,
            storage: None,
        },
        cookie,
        form,
    })
}

/// The query key and value a sign-in's redirect carries the session in,
/// when its address has a parameter whose value looks like one.
fn session_in_redirect(action: &url::Url, answer: &Response) -> Option<(String, String)> {
    let target = action.join(answer.header("location")).ok()?;
    target
        .query_pairs()
        .find(|(_, value)| itda_types::url::session_value(value))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
}

/// The session the first page after the sign-in puts on its links, when a
/// site that set no cookie keeps it in the address: one read of the
/// redirect's target, on the same site, with the cookies the answer set.
async fn session_on_next_page(
    net: &Network,
    scope: &url::Url,
    action: &url::Url,
    answer: &Response,
    interval: Option<Duration>,
) -> Option<(String, String)> {
    let target = action.join(answer.header("location")).ok()?;
    if !same_site_origin(scope, &target) || !matches!(target.scheme(), "http" | "https") {
        return None;
    }
    let mut request = Request::read(target.as_str());
    request.interval = interval;
    let page = net.fetch(request).await.ok()?;
    if !page.successful() || !page.header("content-type").contains("html") {
        return None;
    }
    itda_html::evidence::session_in_links(&String::from_utf8_lossy(&page.bytes), &page.url)
}

/// A cookie as a `Set-Cookie` header gave it: its name, its value, and
/// whether the header deletes it (`Max-Age=0`).
struct SetCookie {
    name: String,
    value: String,
    deleted: bool,
}

fn set_cookies(response: &Response) -> Vec<SetCookie> {
    response
        .headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("set-cookie"))
        .filter_map(|(_, value)| {
            let mut parts = value.split(';');
            let (name, value) = parts.next()?.trim().split_once('=')?;
            let name = name.trim();
            if name.is_empty() || name.contains([' ', ',']) {
                return None;
            }
            let deleted = parts.any(|attribute| {
                let attribute = attribute.trim().to_ascii_lowercase();
                attribute
                    .strip_prefix("max-age=")
                    .is_some_and(|age| age.trim().parse::<i64>().is_ok_and(|age| age <= 0))
            });
            Some(SetCookie {
                name: name.to_owned(),
                value: value.trim().to_owned(),
                deleted,
            })
        })
        .collect()
}

/// The cookies after the sign-in: what the form page set, then what the
/// answer set, later ones replacing earlier ones of the same name, deleted
/// ones dropped.
fn merge(before: Vec<SetCookie>, after: &[SetCookie]) -> Vec<SetCookie> {
    let mut jar: Vec<SetCookie> = Vec::new();
    for cookie in before.into_iter().chain(after.iter().map(|c| SetCookie {
        name: c.name.clone(),
        value: c.value.clone(),
        deleted: c.deleted,
    })) {
        jar.retain(|kept| kept.name != cookie.name);
        if !cookie.deleted {
            jar.push(cookie);
        }
    }
    jar
}

fn header(jar: &[SetCookie]) -> String {
    jar.iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The cookie that shows the sign-in, among those the answer set: one named
/// for a signed-in user or a session, else the first.
fn proof(after: &[SetCookie]) -> Option<String> {
    let kept: Vec<&SetCookie> = after.iter().filter(|c| !c.deleted).collect();
    let named = |words: &[&str]| {
        kept.iter()
            .find(|c| {
                let name = c.name.to_ascii_lowercase();
                words.iter().any(|word| name.contains(word))
            })
            .map(|c| c.name.clone())
    };
    named(&["logged"])
        .or_else(|| named(&["userid", "user_id"]))
        .or_else(|| named(&["session", "sess", "sid"]))
        .or_else(|| kept.first().map(|c| c.name.clone()))
}

#[cfg(test)]
mod tests {
    use super::{Session, SetCookie, merge, proof, session_in_redirect, set_cookies};
    use itda_net::Response;

    #[test]
    fn a_session_in_the_redirect_is_named_and_kept_only_when_the_definition_asks() {
        let answer = Response {
            url: "https://site.example/login".into(),
            status: 303,
            headers: vec![(
                "location".into(),
                "/portal/main.do?sid=9f8e7d6c5b4a3210&next=%2Fnotices".into(),
            )],
            bytes: Vec::new(),
            duration_ms: 1,
            defense: "none".into(),
            file: None,
        };
        let action = url::Url::parse("https://site.example/login").unwrap();
        let query = session_in_redirect(&action, &answer);
        assert_eq!(
            query,
            Some(("sid".to_owned(), "9f8e7d6c5b4a3210".to_owned()))
        );
        let session = Session {
            cookies: "lang=en".into(),
            count: 1,
            query,
            storage: None,
        };
        assert_eq!(session.value_from(Some("lang")), "query:sid");
        assert_eq!(session.secret_for("query:sid").expose(), "9f8e7d6c5b4a3210");
        assert_eq!(session.secret_for("cookie:lang").expose(), "lang=en");
        assert_eq!(session.secret_for("").expose(), "lang=en");
        let plain = Session {
            cookies: "SID=x".into(),
            count: 1,
            query: None,
            storage: None,
        };
        assert_eq!(plain.value_from(Some("SID")), "cookie:SID");
        assert_eq!(plain.value_from(None), "");
        let elsewhere = Response {
            headers: vec![("location".into(), "/notices?page=2".into())],
            ..answer
        };
        assert_eq!(session_in_redirect(&action, &elsewhere), None);
    }

    fn response(set: &[&str]) -> Response {
        Response {
            url: "https://site.example/login".into(),
            status: 303,
            headers: set
                .iter()
                .map(|value| ("set-cookie".to_owned(), (*value).to_owned()))
                .collect(),
            bytes: Vec::new(),
            duration_ms: 1,
            defense: "none".into(),
            file: None,
        }
    }

    #[test]
    fn cookies_from_both_answers_are_merged_and_deleted_ones_dropped() {
        let before = set_cookies(&response(&[
            "_csrf=abc; Path=/; HttpOnly",
            "old=gone; Max-Age=0",
            "sid=first; Path=/",
        ]));
        let after = set_cookies(&response(&["sid=second; Path=/; HttpOnly", "lang=en"]));
        let jar = merge(before, &after);
        let names: Vec<&str> = jar.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["_csrf", "sid", "lang"]);
        assert_eq!(super::header(&jar), "_csrf=abc; sid=second; lang=en");
    }

    #[test]
    fn the_proof_cookie_prefers_a_signed_in_name() {
        let cookie = |name: &str| SetCookie {
            name: name.into(),
            value: "v".into(),
            deleted: false,
        };
        assert_eq!(
            proof(&[cookie("wordpress_test"), cookie("wordpress_logged_in_ab")]).as_deref(),
            Some("wordpress_logged_in_ab")
        );
        assert_eq!(
            proof(&[cookie("my_wiki_session"), cookie("my_wikiUserID")]).as_deref(),
            Some("my_wikiUserID")
        );
        assert_eq!(
            proof(&[cookie("lang"), cookie("i_like_gitea")]).as_deref(),
            Some("lang")
        );
        assert_eq!(proof(&[]), None);
    }
}
