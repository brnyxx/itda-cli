//! Drafts site definitions from descriptions a site already publishes.
//!
//! A draft is a starting point for a person, never something installed
//! silently: every operation carries the evidence it came from, changes stay
//! changes, and whatever could not be translated is listed as a note instead of
//! guessed. The draft is checked with the same rules `itda` applies at install
//! time, so problems show up before anyone runs it.
mod openapi;

pub use openapi::from_openapi;

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
