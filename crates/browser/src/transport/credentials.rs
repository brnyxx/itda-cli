use super::BrowserRequest;
use itda_types::{
    error::{AppError, AppResult},
    url::validate_url,
    util,
};
use std::collections::BTreeMap;
use url::Url;

pub(super) fn action_url(raw: &str) -> AppResult<Url> {
    if raw.len() > 8192 || raw.chars().any(char::is_control) {
        return Err(AppError::api("bad_url", 400));
    }
    let mut url = Url::parse(raw).map_err(|_| AppError::api("bad_url", 400))?;
    let query = url.query().map(str::to_owned);
    url.set_query(None);
    let mut url = validate_url(url.as_str())?;
    url.set_query(query.as_deref());
    Ok(url)
}

pub(super) fn credential_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    util::secret_field(&name)
        || [
            "authorization",
            "cookie",
            "key",
            "session",
            "sessionid",
            "csrf",
            "xsrf",
        ]
        .contains(&name.as_str())
        || [
            "token", "secret", "api-key", "api_key", "apikey", "csrf", "xsrf",
        ]
        .iter()
        .any(|part| name.contains(part))
}

pub(super) fn fingerprint(request: &BrowserRequest, url: &Url) -> AppResult<String> {
    if let Some(value) = &request.credential_fingerprint {
        return Ok(format!("explicit:{value}"));
    }
    let headers: Vec<_> = request
        .headers
        .iter()
        .filter(|(name, _)| credential_name(name))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.as_str()))
        .collect();
    let query: Vec<_> = url
        .query_pairs()
        .filter(|(name, _)| credential_name(name))
        .collect();
    Ok(format!(
        "derived:{}",
        util::hash(&serde_json::to_vec(&(headers, query))?)
    ))
}

pub(super) fn cookies(request: &BrowserRequest) -> AppResult<BTreeMap<String, String>> {
    let mut cookies = BTreeMap::new();
    for (_, value) in request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
    {
        for cookie in value.split(';') {
            let (name, value) = cookie
                .trim()
                .split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .ok_or_else(|| AppError::api("data_invalid", 400))?;
            cookies.insert(name.to_owned(), value.to_owned());
        }
    }
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::BrowserMode;
    use std::time::Duration;

    #[test]
    fn only_credential_revisions_change_context_identity() {
        let mut request = BrowserRequest {
            url: "https://example.test/query?page=1&api_key=first".into(),
            method: "POST".into(),
            headers: vec![("Accept".into(), "text/html".into())],
            body: Some(b"first query".to_vec()),
            account_id: "account".into(),
            credential_fingerprint: None,
            read_only: true,
            limit: 1024,
            timeout: Duration::from_secs(2),
            allow_local: false,
            mode: BrowserMode::Fetch,
            cooldowns: Vec::new(),
        };
        let first = fingerprint(&request, &action_url(&request.url).unwrap()).unwrap();
        request.url = "https://example.test/query?page=2&api_key=first".into();
        request.headers[0].1 = "application/json".into();
        request.body = Some(b"second query".to_vec());
        assert_eq!(
            fingerprint(&request, &action_url(&request.url).unwrap()).unwrap(),
            first
        );
        request.url = "https://example.test/query?page=2&api_key=second".into();
        assert_ne!(
            fingerprint(&request, &action_url(&request.url).unwrap()).unwrap(),
            first
        );
        request.credential_fingerprint = Some(String::new());
        let anonymous = fingerprint(&request, &action_url(&request.url).unwrap()).unwrap();
        request
            .headers
            .push(("Authorization".into(), "Bearer revision".into()));
        assert_eq!(
            fingerprint(&request, &action_url(&request.url).unwrap()).unwrap(),
            anonymous
        );
        request.credential_fingerprint = Some("revision".into());
        assert_ne!(
            fingerprint(&request, &action_url(&request.url).unwrap()).unwrap(),
            anonymous
        );
        request.credential_fingerprint = None;
        request.url = "https://example.test/?api_key=first&api_key=stable".into();
        let duplicate = fingerprint(&request, &action_url(&request.url).unwrap()).unwrap();
        request.url = "https://example.test/?api_key=changed&api_key=stable".into();
        assert_ne!(
            fingerprint(&request, &action_url(&request.url).unwrap()).unwrap(),
            duplicate
        );
    }
}
