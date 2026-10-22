//! Classifies block and challenge responses so they can be reported and paused.
//! Classification only applies to blocking signals; CDN presence headers on 2xx
//! responses are informational.
//!
//! Derived from Hycli `src/defend.rs` (Apache-2.0). Changes: remedies no longer
//! suggest escalating to a browser transport or a live browser session.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    None,
    Cloudflare,
    BotManager,
    RateLimit,
    Captcha,
    Unknown,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::None => "none",
            Class::Cloudflare => "cloudflare",
            Class::BotManager => "bot-manager",
            Class::RateLimit => "rate-limit",
            Class::Captcha => "captcha",
            Class::Unknown => "unknown",
        }
    }

    /// What the user can do. Blocks are reported, never worked around: there is no
    /// automatic switch to a browser or to a different client fingerprint.
    pub fn remedy(self) -> &'static str {
        match self {
            Class::Cloudflare | Class::BotManager | Class::Captcha | Class::Unknown => {
                "The website refused automated access. Requests to it are paused. Check in your browser whether this use is allowed, then resume or use the site's official API."
            }
            Class::RateLimit => {
                "Wait for Retry-After or the reset time before a bounded read retry."
            }
            Class::None => "",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Observation {
    pub status_code: u16,
    pub headers: Vec<(String, String)>, // lowercase keys
    pub body_snippet: String,
}

impl Observation {
    pub fn header(&self, k: &str) -> &str {
        self.headers
            .iter()
            .find(|(hk, _)| hk == k)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    fn is_json_payload(&self) -> bool {
        let ct = self.header("content-type").to_ascii_lowercase();
        let trimmed = self.body_snippet.trim_start();
        ct.contains("application/json")
            || ct.contains("+json")
            || trimmed.starts_with('{')
            || trimmed.starts_with('[')
    }
}

pub fn classify(o: &Observation) -> (Class, &'static str) {
    if o.status_code == 429 || crate::retry_after(&o.headers).is_some() {
        return (Class::RateLimit, "status-429/retry-after");
    }
    if o.status_code < 400 {
        if !o.is_json_payload()
            && o.body_snippet.len() < 32_768
            && body_hit(
                o,
                &[
                    "class=\"g-recaptcha\"",
                    "class=\"h-captcha\"",
                    "class=\"cf-turnstile\"",
                    "data-sitekey=",
                    "captcha-delivery.com",
                    "arkoselabs.com",
                ],
            )
            && body_hit(
                o,
                &[
                    "<title>just a moment",
                    "verify you are human",
                    "security check",
                    "complete the security check",
                    "pardon our interruption",
                    "attention required",
                ],
            )
        {
            return (Class::Captcha, "captcha-interstitial-2xx");
        }
        return (Class::None, "");
    }
    if !o.header("cf-mitigated").is_empty() {
        return (Class::Cloudflare, "cf-mitigated");
    }
    if body_hit(
        o,
        &[
            "just a moment",
            "cf-chl",
            "challenge-platform",
            "/turnstile/",
            "attention required! | cloudflare",
        ],
    ) {
        return (Class::Cloudflare, "challenge-body");
    }
    if body_hit(
        o,
        &[
            "hcaptcha.com",
            "recaptcha/api",
            "geetest",
            "arkoselabs",
            "\"captcha_key\"",
            "\"captcha_sitekey\"",
            "captcha-delivery.com",
        ],
    ) {
        return (Class::Captcha, "captcha-vendor-body");
    }
    if matches!(o.status_code, 400 | 401 | 404 | 405 | 409 | 410 | 422) {
        return (Class::None, "");
    }
    if o.is_json_payload() {
        return (Class::None, "");
    }
    if !o.header("cf-ray").is_empty() || o.header("server") == "cloudflare" {
        return (Class::Cloudflare, "cf-block-headers");
    }
    if o.header("server").contains("akamai")
        || !o.header("x-akamai-transformed").is_empty()
        || !o.header("akamai-ref").is_empty()
    {
        return (Class::BotManager, "akamai-headers");
    }
    if o.status_code == 403 || o.status_code == 503 {
        return (Class::Unknown, "403/503-no-signal");
    }
    (Class::None, "")
}

fn body_hit(o: &Observation, subs: &[&str]) -> bool {
    let b = o.body_snippet.to_lowercase();
    subs.iter().any(|s| b.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdn_presence_on_200_and_json_403_is_not_blocking() {
        let (cls, _) = classify(&Observation {
            status_code: 200,
            headers: vec![
                ("server".into(), "cloudflare".into()),
                ("cf-ray".into(), "x".into()),
            ],
            body_snippet: "Article discussing hcaptcha.com and recaptcha/api in prose".into(),
        });
        assert_eq!(cls, Class::None);
        for status in [401, 403, 404, 405, 410, 422] {
            assert_eq!(
                classify(&Observation {
                    status_code: status,
                    headers: vec![
                        ("server".into(), "cloudflare".into()),
                        ("cf-ray".into(), "synthetic".into()),
                        ("content-type".into(), "application/json".into()),
                    ],
                    body_snippet: "{\"error\":\"not available\"}".into()
                })
                .0,
                Class::None
            );
        }
    }

    #[test]
    fn challenge_body_and_json_captcha_key_are_detected() {
        let (cls, _) = classify(&Observation {
            status_code: 403,
            headers: vec![("server".into(), "cloudflare".into())],
            body_snippet: "Just a moment...".into(),
        });
        assert_eq!(cls, Class::Cloudflare);

        let (cls, _) = classify(&Observation {
            status_code: 400,
            headers: vec![("content-type".into(), "application/json".into())],
            body_snippet: "{\"captcha_key\":[\"You need to update your app\"],\"captcha_sitekey\":\"a9b5fb07\"}".into(),
        });
        assert_eq!(cls, Class::Captcha);
    }

    #[test]
    fn a_retry_after_of_zero_is_not_a_rate_limit() {
        let (cls, _) = classify(&Observation {
            status_code: 200,
            headers: vec![("retry-after".into(), "0".into())],
            body_snippet: "<html>a page</html>".into(),
        });
        assert_eq!(cls, Class::None);
    }

    #[test]
    fn rate_limit() {
        let (cls, _) = classify(&Observation {
            status_code: 429,
            headers: vec![("retry-after".into(), "3".into())],
            body_snippet: String::new(),
        });
        assert_eq!(cls, Class::RateLimit);
    }
}
