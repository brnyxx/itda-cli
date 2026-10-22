//! Exit codes and error output shared by every command.
use itda_types::error::AppError;
use serde_json::json;
use std::process::ExitCode;

/// The exit code contract scripts and agents branch on.
pub fn exit_code(public_code: &str) -> u8 {
    match public_code {
        "ok" => 0,
        "bad_request" | "data_invalid" | "bad_url" | "not_found" | "effect_changed" => 2,
        "auth_required" | "identity_unavailable" => 10,
        "response_mismatch" | "query_failed" | "wrong_media_type" | "partial_answer"
        | "empty_answer" => 11,
        "blocked_by_site" | "unsafe_target" | "local_target" => 12,
        "rate_limited" | "site_rate_limited" => 13,
        "api_denied" => 14,
        "approval_required" | "approval_pending" | "writes_disabled" => 20,
        "approval_expired" => 21,
        "pagination_expired" | "pagination_invalid" => 23,
        "action_outcome_unknown" => 22,
        "request_failed" | "timeout" | "too_large" | "request_lease_lost" => 30,
        _ => 1,
    }
}

/// Report an error on stdout as a JSON envelope, or on stderr for people.
pub fn fail(error: &AppError, as_json: bool) -> ExitCode {
    if as_json {
        let mut envelope = error.extra.clone();
        envelope.insert("status".into(), json!(error.public_code));
        envelope.insert("message".into(), json!(error.msg));
        envelope.insert("remedy".into(), json!(error.remedy));
        println!("{}", serde_json::Value::Object(envelope));
    } else {
        if error.msg == error.public_code {
            eprintln!("error[{}]", error.public_code);
        } else {
            eprintln!("error[{}]: {}", error.public_code, error.msg);
        }
        if !error.remedy.is_empty() {
            eprintln!("  fix: {}", error.remedy);
        }
        if let Some(line) = outcome_check_words(error) {
            eprintln!("  {line}");
        }
    }
    ExitCode::from(exit_code(&error.public_code))
}

/// What the confirming read said after a lost answer, in the person's
/// language; `None` when no read was run.
fn outcome_check_words(error: &AppError) -> Option<String> {
    let check = error.extra.get("outcome_check")?;
    let read = check["read"].as_str().unwrap_or_default();
    let failed = check["error"].as_str().unwrap_or("unknown");
    Some(match check["result"].as_str() {
        Some("landed") => crate::text::tf("outcome_landed", &[("read", read)]),
        Some("not_found") => crate::text::tf("outcome_not_found", &[("read", read)]),
        _ => crate::text::tf("outcome_unknown", &[("read", read), ("error", failed)]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outcome_check_is_said_in_words() {
        let landed = AppError::api("action_outcome_unknown", 502).with(
            "outcome_check",
            json!({"read": "list-notices", "result": "landed"}),
        );
        let words = outcome_check_words(&landed).unwrap();
        assert!(words.contains("list-notices"), "{words}");
        let unknown = AppError::api("action_outcome_unknown", 502).with(
            "outcome_check",
            json!({"read": "list-notices", "result": "unknown", "error": "timeout"}),
        );
        let words = outcome_check_words(&unknown).unwrap();
        assert!(words.contains("timeout"), "{words}");
        assert!(outcome_check_words(&AppError::api("action_outcome_unknown", 502)).is_none());
    }

    #[test]
    fn codes_follow_the_contract() {
        assert_eq!(exit_code("auth_required"), 10);
        assert_eq!(exit_code("response_mismatch"), 11);
        assert_eq!(exit_code("blocked_by_site"), 12);
        assert_eq!(exit_code("site_rate_limited"), 13);
        assert_eq!(exit_code("api_denied"), 14);
        assert_eq!(exit_code("approval_pending"), 20);
        assert_eq!(exit_code("query_failed"), 11);
        assert_eq!(exit_code("effect_changed"), 2);
        assert_eq!(exit_code("writes_disabled"), 20);
        assert_eq!(exit_code("pagination_expired"), 23);
        assert_eq!(exit_code("timeout"), 30);
        assert_eq!(exit_code("wrong_media_type"), 11);
        assert_eq!(exit_code("partial_answer"), 11);
        assert_eq!(exit_code("request_lease_lost"), 30);
        assert_eq!(exit_code("internal"), 1);
    }
}
