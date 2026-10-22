//! Policy layer: decides whether an operation reads or changes a website,
//! whether a SiteSpec is safe to install, records single-use approvals and
//! keeps the page markers behind the handles agents turn pages with.
//! Depends only on `itda-types`, `itda-html`, `itda-spec` and `itda-store`.
pub mod approvals;
pub mod classify;
pub mod cursors;
pub mod validate;

pub use approvals::{Approval, Approvals, ExecutionRequest, ReviewInput};
pub use classify::{dangerous_intent, operation_effect, safe_read_url};
pub use cursors::Cursors;
pub use itda_spec::Effect;
pub use validate::{Problem, overrides_method, spec_problems, validate_spec};
