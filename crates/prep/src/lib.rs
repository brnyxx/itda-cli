//! Drafts site definitions from descriptions a site already publishes.
//!
//! A draft is a starting point for a person, never something installed
//! silently: every operation carries the evidence it came from, changes stay
//! changes, and whatever could not be translated is listed as a note instead of
//! guessed. The draft is checked with the same rules `itda` applies at install
//! time, so problems show up before anyone runs it.
mod har;
mod openapi;

pub use har::from_har;
pub use openapi::from_openapi;

use serde_norway::{Mapping, Value as Yaml};

/// Choices a person makes about a draft.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The site id; taken from the document's title when left out.
    pub name: Option<String>,
    /// Overrides the server URL in the document.
    pub base_url: Option<String>,
    /// Keep only operations whose path, name or tag contains this text.
    pub include: Option<String>,
}

/// A drafted definition and what the person should review.
#[derive(Debug)]
pub struct Draft {
    pub site: String,
    pub yaml: String,
    pub operations: usize,
    pub reads: usize,
    /// What was skipped or simplified, one line each.
    pub notes: Vec<String>,
    /// What the install-time checks found, one line each.
    pub problems: Vec<String>,
}

/// A lower-case id made of letters, digits and single dashes, starting with a
/// letter, as definitions require.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in text.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            out.push('-');
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.ends_with('-') {
                out.push('-');
            }
            previous_lower = false;
        }
    }
    let trimmed = out
        .trim_matches('-')
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
    let mut id: String = trimmed.chars().take(64).collect();
    while id.ends_with('-') {
        id.pop();
    }
    id
}

/// Writes the definition and checks it with the rules `itda` applies at
/// install time. `source` says what drafted it, for the header comment.
pub(crate) fn finish(
    site: &str,
    title: &str,
    base_url: &str,
    auth: Vec<Yaml>,
    operations: Vec<Mapping>,
    notes: Vec<String>,
    source: &str,
) -> Draft {
    let mut site_map = Mapping::new();
    site_map.insert("name".into(), site.into());
    if !title.is_empty() {
        site_map.insert("title".into(), title.into());
    }
    site_map.insert("base_url".into(), base_url.into());
    let mut root = Mapping::new();
    root.insert("spec_version".into(), 1.into());
    root.insert("site".into(), Yaml::Mapping(site_map));
    if !auth.is_empty() {
        root.insert("auth".into(), Yaml::Sequence(auth));
    }
    let reads = operations
        .iter()
        .filter(|op| op.get("effect").and_then(Yaml::as_str) == Some("read"))
        .count();
    let count = operations.len();
    root.insert(
        "operations".into(),
        Yaml::Sequence(operations.into_iter().map(Yaml::Mapping).collect()),
    );
    let body = serde_norway::to_string(&Yaml::Mapping(root)).unwrap_or_default();
    let yaml = format!(
        "# Drafted by {source}. Review every operation before use:\n# check names, effects and response checks, and remove what agents should not call.\n{body}"
    );
    let problems = match itda_spec::parse(yaml.as_bytes()) {
        Ok(spec) => itda_policy::spec_problems(&spec)
            .into_iter()
            .map(|p| format!("{}: {}", p.field, p.detail))
            .collect(),
        Err(error) => vec![error.msg],
    };
    Draft {
        site: site.to_owned(),
        yaml,
        operations: count,
        reads,
        notes,
        problems,
    }
}

pub(crate) fn insert_after(map: &mut Mapping, after: &str, key: &str, value: Yaml) {
    let mut out = Mapping::new();
    for (k, v) in std::mem::take(map) {
        let matched = k.as_str() == Some(after);
        out.insert(k, v);
        if matched {
            out.insert(key.into(), value.clone());
        }
    }
    *map = out;
}

/// `name` first and `evidence` after `effect`, as people write them.
pub(crate) fn reorder(entry: Mapping) -> Mapping {
    let order = [
        "name",
        "desc",
        "method",
        "path",
        "auth",
        "effect",
        "evidence",
        "preflight",
        "params",
        "query",
        "body",
        "response",
    ];
    let mut out = Mapping::new();
    for key in order {
        if let Some(value) = entry.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slugs_are_valid_ids() {
        assert_eq!(slug("listPetsByOwner"), "list-pets-by-owner");
        assert_eq!(slug("GET /v1/pets/{id}"), "get-v1-pets-id");
        assert_eq!(slug("2024 Store API"), "store-api");
        assert_eq!(slug("HTTPServer"), "httpserver");
        assert!(itda_types::util::valid_id(&slug("Pet_Store  API!!")));
    }
}
