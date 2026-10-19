//! Policy layer: decides whether an operation reads or changes a website,
//! whether a SiteSpec is safe to install, and records single-use approvals.
//! Depends only on `itda-types`, `itda-html` and `itda-spec`.
pub mod approvals;
pub mod classify;
pub mod validate;

pub use approvals::{Approval, Approvals, ExecutionRequest, ReviewInput};
pub use classify::{dangerous_intent, operation_effect, safe_read_url};
pub use itda_spec::Effect;
pub use validate::{allowed_header, validate_spec};
