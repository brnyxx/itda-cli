//! What `itda setup` says, in the person's language. Keys are what the
//! code asks for; English is the fallback for every key. The strings live in
//! `../text/<tag>.json`, one file per language, listed in [`SUPPORTED`]. See
//! `docs/i18n.md` for the rules and the glossary.
use std::collections::HashMap;
use std::sync::OnceLock;

/// The languages itda speaks, as the tag `lang()` returns. English first:
/// it is the source and the fallback.
pub const SUPPORTED: [(&str, &str); 12] = [
    ("en", include_str!("../text/en.json")),
    ("ko", include_str!("../text/ko.json")),
    ("ja", include_str!("../text/ja.json")),
    ("zh-Hans", include_str!("../text/zh-Hans.json")),
    ("zh-Hant", include_str!("../text/zh-Hant.json")),
    ("es", include_str!("../text/es.json")),
    ("fr", include_str!("../text/fr.json")),
    ("de", include_str!("../text/de.json")),
    ("pt-BR", include_str!("../text/pt-BR.json")),
    ("vi", include_str!("../text/vi.json")),
    ("id", include_str!("../text/id.json")),
    ("ru", include_str!("../text/ru.json")),
];

/// The language tag `ITDA_LANG` or `LANG` resolves to, else `en`.
pub fn lang() -> &'static str {
    static LANG: OnceLock<&'static str> = OnceLock::new();
    LANG.get_or_init(|| resolve(&crate::lang()))
}

/// The supported tag a locale names, else English; see
/// [`itda_types::lang::supported`].
pub fn resolve(locale: &str) -> &'static str {
    itda_types::lang::resolve(locale)
}

fn table(tag: &str) -> &'static HashMap<String, String> {
    static TABLES: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        SUPPORTED
            .iter()
            .map(|(tag, json)| {
                let table = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("apps/cli/text/{tag}.json: {e}"));
                (*tag, table)
            })
            .collect()
    });
    &tables[tag]
}

pub fn t(key: &str) -> &'static str {
    let own = table(lang()).get(key);
    own.or_else(|| table("en").get(key))
        .map(String::as_str)
        .unwrap_or("?")
}

/// How people say the client's name, from the slug `itda mcp install` takes.
pub fn client_label(slug: &str) -> &str {
    match slug {
        "claude-desktop" => "Claude Desktop",
        "claude-code" => "Claude Code",
        "cursor" => "Cursor",
        "vscode" => "VS Code",
        "codex" => "Codex",
        other => other,
    }
}

/// What a site's API description added to a draft, in one line.
pub fn described(described: &itda_prep::Described) -> String {
    tf(
        "described",
        &[
            ("url", &described.url),
            ("reads", &described.reads.to_string()),
            ("changes", &described.changes.to_string()),
        ],
    )
}

/// The frames a page's content was read through, in one line; `None` when
/// none was followed.
pub fn followed_frames(followed: &[String]) -> Option<String> {
    let list = followed.join(", ");
    match followed.len() {
        0 => None,
        1 => Some(tf("followed_frame", &[("list", &list)])),
        n => Some(tf(
            "followed_frames",
            &[("n", &n.to_string()), ("list", &list)],
        )),
    }
}

/// `t(key)` with `{name}` fields filled in.
pub fn tf(key: &str, fields: &[(&str, &str)]) -> String {
    let mut out = t(key).to_owned();
    for (name, value) in fields {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn tables() -> Vec<(&'static str, HashMap<String, String>)> {
        SUPPORTED
            .iter()
            .map(|(tag, json)| {
                let table: HashMap<String, String> = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("apps/cli/text/{tag}.json: {e}"));
                (*tag, table)
            })
            .collect()
    }

    fn fields(text: &str) -> BTreeSet<&str> {
        text.match_indices('{')
            .filter_map(|(i, _)| text[i + 1..].split('}').next())
            .filter(|name| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .collect()
    }

    #[test]
    fn every_language_has_every_key() {
        let tables = tables();
        let english: BTreeSet<&str> = tables[0].1.keys().map(String::as_str).collect();
        assert!(english.len() > 100, "english table lost its keys");
        for (tag, table) in &tables[1..] {
            let own: BTreeSet<&str> = table.keys().map(String::as_str).collect();
            let missing: Vec<_> = english.difference(&own).collect();
            let extra: Vec<_> = own.difference(&english).collect();
            assert!(missing.is_empty(), "{tag}.json lacks {missing:?}");
            assert!(
                extra.is_empty(),
                "{tag}.json has keys english lacks: {extra:?}"
            );
            for (key, value) in table {
                assert!(!value.trim().is_empty(), "{tag}.json: {key} is empty");
            }
        }
    }

    #[test]
    fn every_translation_keeps_the_fields() {
        let tables = tables();
        for (key, english) in &tables[0].1 {
            let wanted = fields(english);
            for (tag, table) in &tables[1..] {
                let got = fields(&table[key]);
                assert_eq!(got, wanted, "{tag}.json: {key} fills in different fields");
            }
        }
    }

    #[test]
    fn locales_resolve_to_supported_tags() {
        for (locale, tag) in [
            ("", "en"),
            ("C", "en"),
            ("POSIX", "en"),
            ("en_US.UTF-8", "en"),
            ("ko", "ko"),
            ("ko_KR.UTF-8", "ko"),
            ("ja_JP.UTF-8", "ja"),
            ("ja-JP", "ja"),
            ("zh", "zh-Hans"),
            ("zh_CN.UTF-8", "zh-Hans"),
            ("zh-SG", "zh-Hans"),
            ("zh-Hans-CN", "zh-Hans"),
            ("zh_TW.Big5", "zh-Hant"),
            ("zh-HK", "zh-Hant"),
            ("zh-Hant", "zh-Hant"),
            ("zh-hant-tw", "zh-Hant"),
            ("pt", "pt-BR"),
            ("pt_PT.UTF-8", "pt-BR"),
            ("pt-BR", "pt-BR"),
            ("es_MX.UTF-8", "es"),
            ("fr_CA", "fr"),
            ("de_DE.UTF-8@euro", "de"),
            ("vi_VN", "vi"),
            ("id_ID", "id"),
            ("in", "id"),
            ("ru_RU.UTF-8", "ru"),
            ("sv_SE.UTF-8", "en"),
            ("tlh", "en"),
        ] {
            assert_eq!(resolve(locale), tag, "{locale:?}");
        }
    }

    #[test]
    fn the_strings_cover_the_languages_drafts_are_written_in() {
        let tags: Vec<&str> = SUPPORTED.iter().map(|(tag, _)| *tag).collect();
        assert_eq!(tags, itda_types::lang::LANGUAGES);
    }

    #[test]
    fn supported_tags_resolve_to_themselves() {
        for (tag, _) in SUPPORTED {
            assert_eq!(resolve(tag), tag);
            assert_eq!(resolve(&tag.to_ascii_uppercase()), tag);
        }
    }
}
