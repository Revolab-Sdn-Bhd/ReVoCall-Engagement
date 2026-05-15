//! Port-specific error types.
//!
//! Each error enum corresponds to one port and has three variants:
//! - `Transient` — retriable error (network timeout, temporary unavailability)
//! - `Permanent` — non-retriable error (invalid input, not found, permission denied)
//! - `Unavailable` — the downstream service is structurally unreachable

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum JmError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum PostCallError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalyticsError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
}
