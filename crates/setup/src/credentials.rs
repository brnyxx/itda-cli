//! What a recording signed in with, offered for the keyring: a header token
//! for each header `auth` entry, and the browser session for a `browser` one.
//! Values are never shown, and nothing is kept without a person's yes.
use crate::Context;
use itda_spec::Spec;
use itda_types::error::{AppResult, usage};
use itda_vault::{KeyringStore, Secret};
use serde_json::Value;

/// One credential the recording used, named for the person; the value stays
/// inside.
pub struct Offer {
    /// "the Authorization header the pages sent", "the browser session (3 cookies)".
    pub what: String,
    /// The secret's name in the vault.
    pub name: String,
    pub secret: Secret,
}

/// The credentials the recording used that the definition's `auth` entries
/// refer to.
pub fn offers(spec: &Spec, har: &Value, cookies: &[itda_browser::Cookie]) -> Vec<Offer> {
    let site = &spec.site.name;
    let mut offers = Vec::new();
    for auth in &spec.auth {
        match auth.kind.as_str() {
            "header" => {
                let Some(name) = auth.value_from.strip_prefix(&format!("store:{site}/")) else {
                    continue;
                };
                if let Some(value) =
                    header_value(har, &spec.site.base_url, &auth.header, &auth.prefix)
                {
                    offers.push(Offer {
                        what: format!("the {} header the pages sent", auth.header),
                        name: name.to_owned(),
                        secret: Secret::new(value),
                    });
                }
            }
            "browser" if !cookies.is_empty() => offers.push(Offer {
                what: format!("the browser session ({} cookies)", cookies.len()),
                name: itda_runtime::SESSION_SECRET.to_owned(),
                secret: Secret::new(itda_browser::header(cookies)),
            }),
            _ => {}
        }
    }
    offers
}

/// Whether secrets can be written at all: `ITDA_SECRET_STORE=env` reads
/// `ITDA_SECRET__` variables and never writes.
pub fn store_is_env() -> bool {
    std::env::var("ITDA_SECRET_STORE").is_ok_and(|v| v == "env")
}

/// Refuse early when a sign-in could not be kept.
pub fn writable() -> AppResult<()> {
    if store_is_env() {
        return Err(usage(
            "Environment secrets are read-only",
            "Sign in where the system keyring is available, or set the session in an ITDA_SECRET__ variable.",
        ));
    }
    KeyringStore::available()
}

/// Keep one credential as `site/account` and lift any pause on the site, so
/// the account may run again.
pub async fn keep(
    ctx: &Context,
    site: &str,
    account: &str,
    name: &str,
    secret: Secret,
) -> AppResult<()> {
    ctx.vault.add_account(site, account, "").await?;
    ctx.vault.set_secret(site, account, name, secret).await?;
    crate::resume(ctx, site, Some(account)).await
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
