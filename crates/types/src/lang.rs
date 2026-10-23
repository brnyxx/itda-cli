//! The languages itda speaks to a person, and the one a locale names. See
//! `docs/i18n.md` for the table this follows.

/// Every language tag, English first: it is the source and the fallback.
pub const LANGUAGES: [&str; 12] = [
    "en", "ko", "ja", "zh-Hans", "zh-Hant", "es", "fr", "de", "pt-BR", "vi", "id", "ru",
];

/// The tag a locale names: BCP 47 (`ja`, `zh-TW`, `pt-BR`) or POSIX
/// (`ja_JP.UTF-8`, `zh_TW.Big5`). Encoding and modifier are dropped; case
/// does not matter. Traditional Chinese for Taiwan, Hong Kong and Macao,
/// Simplified for the rest of `zh`; every Portuguese is `pt-BR`; `in` is the
/// old code for Indonesian. `None` for `C`, `POSIX`, an empty locale and a
/// language itda does not speak.
pub fn supported(locale: &str) -> Option<&'static str> {
    let tag = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();
    let mut parts = tag.split('-').filter(|p| !p.is_empty());
    let language = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    match language {
        "zh" => {
            let traditional = rest
                .iter()
                .any(|p| matches!(*p, "tw" | "hk" | "mo") || p.starts_with("hant"));
            Some(if traditional { "zh-Hant" } else { "zh-Hans" })
        }
        "pt" => Some("pt-BR"),
        "in" => Some("id"),
        other => LANGUAGES
            .iter()
            .find(|tag| tag.eq_ignore_ascii_case(other))
            .copied(),
    }
}

/// The tag a locale names, else English.
pub fn resolve(locale: &str) -> &'static str {
    supported(locale).unwrap_or("en")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locale_names_a_language_or_none() {
        assert_eq!(supported("ja_JP.UTF-8"), Some("ja"));
        assert_eq!(supported("zh-HK"), Some("zh-Hant"));
        assert_eq!(supported("pt_PT"), Some("pt-BR"));
        for none in ["", "C", "POSIX", "sv_SE.UTF-8"] {
            assert_eq!(supported(none), None, "{none:?}");
        }
        assert_eq!(resolve("C"), "en");
        for tag in LANGUAGES {
            assert_eq!(supported(tag), Some(tag));
        }
    }
}
