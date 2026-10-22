//! Preflight steps: reads that run just before an action to pick up values
//! the site hands out per visit, such as a CSRF token in a form page. A value
//! that step `form` extracts as `csrf` fills `{{step.form.csrf}}` in the
//! action's headers, query and body; the agent never sees or supplies it.
use itda_html::{Field, parse_document};
use itda_net::{Network, Request, Response};
use itda_spec::{Operation, PreflightStep, Spec, interpolate_template_str};
use itda_types::{
    error::{AppError, AppResult, usage},
    json::resolve_pointer,
    url::validate_url,
    util,
};
use std::collections::BTreeMap;

/// What the steps found: the values they extracted, keyed
/// `step.<step>.<name>`, and the cookies their answers set, which go along
/// with the action the way a browser would send them (a Rails site, for
/// one, keeps the token it hands out in a session cookie it rewrites on
/// every answer). They are used for this one request and never kept.
pub(crate) struct Found {
    pub values: BTreeMap<String, String>,
    pub cookies: Vec<(String, String)>,
}

/// Run `op`'s steps in order. Each step is a GET to the origin of
/// `destination`, sent with `headers` (the action's own credential and
/// default headers) and the cookies earlier steps set.
pub(crate) async fn run(
    net: &Network,
    spec: &Spec,
    op: &Operation,
    args: &BTreeMap<String, String>,
    headers: &[(String, String)],
    account_id: &str,
    destination: &url::Url,
) -> AppResult<Found> {
    let mut found = BTreeMap::new();
    let mut cookies: Vec<(String, String)> = Vec::new();
    for step in &op.preflight {
        let mut known = args.clone();
        known.extend(found.clone());
        let url = step_url(spec, op, step, &known)?;
        // The action's credential goes along, so a step stays on its origin.
        if validate_url(&url)?.origin() != destination.origin() {
            return Err(AppError::api("unsafe_target", 400));
        }
        let mut request = Request::read(url);
        request.headers = with_cookies(
            headers
                .iter()
                .filter(|(name, _)| !name.eq_ignore_ascii_case("content-type"))
                .cloned()
                .collect(),
            &cookies,
        );
        request.account_id = account_id.to_owned();
        request.interval = spec.defaults.interval();
        let response = net.fetch(request).await?;
        if !response.successful() {
            return Err(
                AppError::api("response_mismatch", 502).with("preflight", step.name.clone())
            );
        }
        for (name, value) in set_cookies(&response) {
            cookies.retain(|(kept, _)| *kept != name);
            cookies.push((name, value));
        }
        for (name, source) in &step.extract {
            let value = extract(&response, source).ok_or_else(|| {
                let mut error =
                    AppError::api("response_mismatch", 502).with("preflight", step.name.clone());
                error.msg = format!("Preflight step {} found no {name}", step.name);
                error
            })?;
            found.insert(format!("step.{}.{name}", step.name), value);
        }
    }
    Ok(Found {
        values: found,
        cookies,
    })
}

/// The cookies an answer set, by name, without the ones it deleted.
fn set_cookies(response: &Response) -> Vec<(String, String)> {
    response
        .headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("set-cookie"))
        .filter_map(|(_, value)| {
            let mut parts = value.split(';');
            let (name, value) = parts.next()?.trim().split_once('=')?;
            let deleted = parts.any(|attribute| {
                attribute
                    .trim()
                    .to_ascii_lowercase()
                    .strip_prefix("max-age=")
                    .is_some_and(|age| age.trim().parse::<i64>().is_ok_and(|age| age <= 0))
            });
            (!name.trim().is_empty() && !deleted)
                .then(|| (name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// `headers` with `cookies` added to its `cookie` header, each replacing a
/// cookie of the same name the session already sends.
pub(crate) fn with_cookies(
    mut headers: Vec<(String, String)>,
    cookies: &[(String, String)],
) -> Vec<(String, String)> {
    if cookies.is_empty() {
        return headers;
    }
    let mut jar: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .flat_map(|(_, value)| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect();
    for (name, value) in cookies {
        jar.retain(|(kept, _)| kept != name);
        jar.push((name.clone(), value.clone()));
    }
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("cookie"));
    headers.push((
        "cookie".into(),
        jar.iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; "),
    ));
    headers
}

fn step_url(
    spec: &Spec,
    op: &Operation,
    step: &PreflightStep,
    known: &BTreeMap<String, String>,
) -> AppResult<String> {
    if !matches!(step.method.to_ascii_uppercase().as_str(), "" | "GET") {
        return Err(usage(
            format!("Preflight step {} is not a GET", step.name),
            "Preflight steps only read; make the step a GET.",
        ));
    }
    let fill = |text: &str| interpolate_template_str(text, &BTreeMap::new(), known);
    if !step.op.is_empty() {
        let target = spec.op(&step.op)?;
        let args = step
            .args
            .iter()
            .map(|(k, v)| (k.clone(), fill(v)))
            .collect();
        return Ok(spec.build_request(target, &args)?.0);
    }
    let base = [&step.base_url, &op.base_url, &spec.site.base_url]
        .into_iter()
        .find(|base| !base.is_empty())
        .map(|base| base.trim_end_matches('/'))
        .unwrap_or_default();
    Ok(format!("{base}{}", fill(&step.path)))
}

/// One value from a step's answer: `json:/pointer`, `css:selector` or
/// `css:selector@attribute`, `header:name`, `cookie:name` or
/// `ssr:key#/pointer` (a value in the JSON the page embeds for its
/// scripts, such as a build id in `__NEXT_DATA__`). A value must be a
/// string, number or boolean; an object or a list is not a value.
pub(crate) fn extract(response: &Response, source: &str) -> Option<String> {
    let value = if let Some(pointer) = source.strip_prefix("json:") {
        resolve_pointer(&response.json()?, pointer).map(util::plain)
    } else if let Some((key, pointer)) = source
        .strip_prefix("ssr:")
        .and_then(|rest| rest.split_once('#'))
    {
        let html = String::from_utf8_lossy(&response.bytes);
        itda_html::extract_ssr_json(&html, key, pointer)
            .filter(|value| value.is_string() || value.is_number() || value.is_boolean())
            .map(|value| util::plain(&value))
    } else if let Some(selector) = source.strip_prefix("css:") {
        let field = html_field(selector);
        field.extract(&parse_document(&String::from_utf8_lossy(&response.bytes)))
    } else if let Some(name) = source.strip_prefix("header:") {
        Some(response.header(name).to_owned())
    } else if let Some(name) = source.strip_prefix("cookie:") {
        response.cookie_value(name)
    } else {
        None
    }?;
    (!value.is_empty()).then_some(value)
}

fn html_field(source: &str) -> Field {
    match source.rsplit_once('@') {
        Some((selector, attribute))
            if !attribute.is_empty()
                && attribute
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':')) =>
        {
            Field {
                selector: selector.trim().to_owned(),
                attribute: attribute.to_owned(),
            }
        }
        _ => Field {
            selector: source.trim().to_owned(),
            attribute: String::new(),
        },
    }
}
