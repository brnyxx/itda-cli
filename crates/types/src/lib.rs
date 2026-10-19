//! Bottom layer of the workspace. Every other crate may depend on this one;
//! this crate depends on no other workspace crate.
pub mod error;
pub mod url;
pub mod util;

pub use error::{AppError, AppResult, Code};
