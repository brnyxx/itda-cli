//! Error contract with consistent exit codes and actionable remedies.
//!
//! Derived from Hycli `src/apperr.rs` (Apache-2.0). Changes: remedies no longer
//! name a product or command, the `rusqlite` conversion moved behind the `sqlite`
//! feature, and the HTTP response conversion was dropped until a server crate exists.
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Ok,
    Runtime,
    Usage,
    Auth,
    Blocked,
    Spec,
}

impl Code {
    pub fn as_i32(self) -> i32 {
        match self {
            Code::Ok => 0,
            Code::Runtime => 1,
            Code::Usage => 2,
            Code::Auth => 3,
            Code::Blocked => 4,
            Code::Spec => 5,
        }
    }
}

#[derive(Debug)]
pub struct AppError {
    pub code: Code,
    pub msg: String,
    pub remedy: String,
    pub public_code: String,
    pub http_status: u16,
}

impl AppError {
    pub fn new(code: Code, msg: impl Into<String>, remedy: impl Into<String>) -> Self {
        let (public_code, http_status) = match code {
            Code::Ok => ("ok", 200),
            Code::Runtime => ("internal", 500),
            Code::Usage => ("bad_request", 400),
            Code::Auth => ("auth_required", 401),
            Code::Blocked => ("blocked_by_site", 429),
            Code::Spec => ("data_invalid", 400),
        };
        Self {
            code,
            msg: msg.into(),
            remedy: remedy.into(),
            public_code: public_code.into(),
            http_status,
        }
    }

    /// An error with a stable public code. The remedy is chosen from the code.
    pub fn api(public_code: &str, http_status: u16) -> Self {
        let code = match http_status {
            400 | 404 | 409 | 413 => Code::Usage,
            401 => Code::Auth,
            403 | 429 => Code::Blocked,
            _ => Code::Runtime,
        };
        Self {
            code,
            msg: public_code.into(),
            remedy: remedy(public_code).into(),
            public_code: public_code.into(),
            http_status,
        }
    }
}

fn remedy(public_code: &str) -> &'static str {
    match public_code {
        "auth_required" | "identity_unavailable" => {
            "Reconnect the website account or configure its website API key."
        }
        "query_failed" => {
            "The website reported query errors. Check the inputs and the action definition before retrying."
        }
        "response_mismatch" => {
            "The website returned a different response than this action expects. Reconnect if signed out, or review the site definition."
        }
        "site_rate_limited" | "rate_limited" => {
            "Wait for the website's request limit to clear before retrying."
        }
        "blocked_by_site" => {
            "The website refused this request. Open it in your browser and check whether access is allowed; requests stay paused until you resume them."
        }
        "writes_disabled" => "Changes are off for this website. Only the user can allow them.",
        "approval_required" | "approval_expired" => {
            "Run the action again and review its exact inputs before approving."
        }
        "action_outcome_unknown" => {
            "Check the website before trying again; the change may already have happened."
        }
        "unsafe_target" | "local_target" => {
            "Check the destination. Local or private addresses need an explicit allow-local setting."
        }
        "request_failed" | "timeout" => {
            "Check the website and your connection, then retry the read."
        }
        _ => "",
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)?;
        if !self.remedy.is_empty() {
            write!(f, " (remedy: {})", self.remedy)?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        runtime(
            error,
            "Local database operation failed",
            "Check the data directory and try again.",
        )
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        runtime(
            error,
            "Local file operation failed",
            "Check the file and its permissions.",
        )
    }
}

impl From<serde_json::Error> for AppError {
    fn from(_: serde_json::Error) -> Self {
        Self::api("data_invalid", 400)
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Wrap an arbitrary error into a Runtime error.
pub fn runtime<E: std::error::Error>(
    err: E,
    msg: impl Into<String>,
    remedy: impl Into<String>,
) -> AppError {
    AppError::new(Code::Runtime, format!("{} ({})", msg.into(), err), remedy)
}

pub fn spec(msg: impl Into<String>, remedy: impl Into<String>) -> AppError {
    AppError::new(Code::Spec, msg, remedy)
}

pub fn usage(msg: impl Into<String>, remedy: impl Into<String>) -> AppError {
    AppError::new(Code::Usage, msg, remedy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_codes_map_to_exit_codes_and_remedies() {
        let error = AppError::api("auth_required", 401);
        assert_eq!(error.code, Code::Auth);
        assert_eq!(error.code.as_i32(), 3);
        assert!(!error.remedy.is_empty());
        assert_eq!(AppError::api("rate_limited", 429).code, Code::Blocked);
        assert_eq!(AppError::api("not_found", 404).code, Code::Usage);
        assert_eq!(AppError::api("internal", 500).code, Code::Runtime);
    }

    #[test]
    fn remedies_do_not_name_a_product() {
        for code in [
            "auth_required",
            "blocked_by_site",
            "writes_disabled",
            "unsafe_target",
        ] {
            let text = AppError::api(code, 400).remedy.to_ascii_lowercase();
            assert!(!text.contains("hycli"), "{code}");
        }
    }
}
