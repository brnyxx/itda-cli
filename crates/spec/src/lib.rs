//! Declarative SiteSpec format (v1 & v2) for turning websites into typed actions.
//!
//! Derived from Hycli `src/spec.rs` (Apache-2.0). Changes: `Effect` moved here from
//! the policy module so the spec no longer depends on policy; response checks take
//! any `ResponseLike` instead of the network layer's response type; HTML types
//! come from `itda-html`. The format itself is unchanged.
use serde::{Deserialize, Serialize};

mod model;
mod request;
mod validate;

pub use model::*;
pub use request::{FormEncoder, interpolate_template_str, render_json_template};

/// What running an operation does to the website. `Unknown` is treated like a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Read,
    Write,
    #[default]
    Unknown,
}
impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Unknown => "unknown",
        }
    }
}

pub const SPEC_VERSION: i64 = 1;
