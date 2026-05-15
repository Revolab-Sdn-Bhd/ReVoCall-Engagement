//! engagement-hub-ports — port trait signatures and domain types.

#[allow(dead_code)]
pub mod types; // fields populated in T1-03
pub use types::*;

pub mod error;
pub use error::*;

pub mod ports;
pub use ports::*;

#[cfg(feature = "fake")]
pub mod fake;
