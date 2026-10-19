//! Declarative SiteSpec format (v1 & v2) for turning websites into typed actions.
//!
//! Derived from Hycli `src/spec.rs` (Apache-2.0). Changes: `Effect` moved here from
//! the policy module so the spec no longer depends on policy; response checks take
//! any [`ResponseLike`] instead of the network layer's response type; HTML types
//! come from `itda-html`. The format itself is unchanged.
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod model;
mod request;
mod validate;
mod verify;

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

/// The parts of an HTTP response a SiteSpec can verify. Implemented by the
/// network layer; tests implement it with plain data.
pub trait ResponseLike {
    fn url(&self) -> &str;
    fn status(&self) -> u16;
    /// Case-insensitive header lookup; empty when absent.
    fn header(&self, key: &str) -> &str;
    fn body(&self) -> &[u8];
    /// 2xx and not classified as a block or challenge.
    fn successful(&self) -> bool;
    fn json(&self) -> Option<Value> {
        serde_json::from_slice(self.body()).ok()
    }
}

pub const SPEC_VERSION: i64 = 1;
