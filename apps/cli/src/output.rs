//! Exit codes and error output shared by every command.
use itda_types::error::AppError;
use serde_json::json;
use std::process::ExitCode;

/// The exit code contract scripts and agents branch on.
pub fn exit_code(public_code: &str) -> u8 {
    match public_code {
        "ok" => 0,
        "bad_request" | "data_invalid" | "bad_url" | "not_found" => 2,
        "auth_required" | "identity_unavailable" => 10,
        "response_mismatch" => 11,
        "blocked_by_site" | "unsafe_target" | "local_target" => 12,
        "rate_limited" | "site_rate_limited" => 13,
        "approval_required" | "writes_disabled" => 20,
        "approval_expired" => 21,
        "request_failed" | "timeout" | "too_large" => 30,
        _ => 1,
    }
}

/// Report an error on stdout as a JSON envelope, or on stderr for people.
pub fn fail(error: &AppError, as_json: bool) -> ExitCode {
    if as_json {
        let envelope = json!({
            "status": error.public_code,
            "message": error.msg,
            "remedy": error.remedy,
        });
        println!("{envelope}");
    } else {
        if error.msg == error.public_code {
            eprintln!("error[{}]", error.public_code);
        } else {
            eprintln!("error[{}]: {}", error.public_code, error.msg);
        }
        if !error.remedy.is_empty() {
            eprintln!("  fix: {}", error.remedy);
        }
    }
    ExitCode::from(exit_code(&error.public_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_follow_the_contract() {
        assert_eq!(exit_code("auth_required"), 10);
        assert_eq!(exit_code("response_mismatch"), 11);
        assert_eq!(exit_code("blocked_by_site"), 12);
        assert_eq!(exit_code("site_rate_limited"), 13);
        assert_eq!(exit_code("writes_disabled"), 20);
        assert_eq!(exit_code("timeout"), 30);
        assert_eq!(exit_code("internal"), 1);
    }
}
