//! engagement-hub-ports — port trait signatures and domain types.

pub mod types;
pub use types::*;

pub mod error;
pub use error::*;

pub mod ports;
pub use ports::*;

#[cfg(feature = "fake")]
pub mod fake;
