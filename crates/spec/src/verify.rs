//! Checks that a response is the one the operation declared, not a login page or
//! an error page that happens to return 200.
use serde_json::Value;

use crate::ResponseLike;
use crate::model::Operation;

impl Operation {
    pub fn parse_json_response(&self, response: &impl ResponseLike) -> Option<Value> {
        let expected = self.response.as_ref();
        if let Some(exp) = expected
            && !exp.ssr_json.is_empty()
        {
            let html = String::from_utf8_lossy(response.body());
            return itda_html::extract_ssr_json(&html, &exp.ssr_json, &exp.extract_pointer);
        }
        let mut parsed = response.json().or_else(|| {
            let text = std::str::from_utf8(response.body()).ok()?.trim_start();
            let stripped = if let Some(exp) = expected {
                if !exp.strip_prefix.is_empty() {
                    text.strip_prefix(exp.strip_prefix.as_str())
                        .map(str::trim_start)
                } else {
                    None
                }
            } else {
                None
            }
            .or_else(|| {
                for prefix in [")]}',", ")]}'", "while(1);", "for(;;);"] {
                    if let Some(rest) = text.strip_prefix(prefix) {
                        return Some(rest.trim_start());
                    }
                }
                None
            })?;
            serde_json::from_str(stripped).ok()
        })?;
        if let Some(exp) = expected
            && !exp.extract_pointer.is_empty()
        {
            parsed = itda_types::json::resolve_pointer(&parsed, &exp.extract_pointer)?.clone();
        }
        Some(parsed)
    }
    pub fn matches_response(&self, response: &impl ResponseLike) -> bool {
        if !response.successful() {
            return false;
        }
        let Some(expected) = &self.response else {
            return true;
        };
        match expected.format.as_str() {
            "json" => self.parse_json_response(response).is_some_and(|body| {
                expected
                    .required_pointers
                    .iter()
                    .all(|p| itda_types::json::resolve_pointer(&body, p).is_some())
            }),
            "html" => {
                if !expected.ssr_json.is_empty() {
                    return response.header("content-type").contains("text/html")
                        && self.parse_json_response(response).is_some_and(|body| {
                            expected
                                .required_pointers
                                .iter()
                                .all(|p| itda_types::json::resolve_pointer(&body, p).is_some())
                        });
                }
                (!expected.required_html_fields.is_empty() || expected.html.is_some())
                    && response.header("content-type").contains("text/html")
                    && !response.body().is_empty()
                    && {
                        let document =
                            itda_html::parse_document(&String::from_utf8_lossy(response.body()));
                        expected
                            .required_html_fields
                            .iter()
                            .all(|field| field.is_present(&document))
                            && expected.html.as_ref().is_none_or(|extraction| {
                                extraction
                                    .extract(
                                        &String::from_utf8_lossy(response.body()),
                                        response.url(),
                                    )
                                    .is_some()
                            })
                    }
            }
            "text" => {
                response.header("content-type").starts_with("text/") && !response.body().is_empty()
            }
            "xml" => response.header("content-type").contains("xml") && !response.body().is_empty(),
            "empty" => response.body().is_empty(),
            _ => false,
        }
    }
}
