/// Implemented by error types whose variants can be retried.
pub trait IsRetryable {
    fn is_retryable(&self) -> bool;
}

/// Implemented by error types that can represent a caught adapter panic.
pub trait FromPanic {
    fn from_panic() -> Self;
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
    #[error("internal panic in adapter")]
    InternalPanic,
}

impl IsRetryable for RegistryError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Unavailable)
    }
}
impl FromPanic for RegistryError {
    fn from_panic() -> Self {
        Self::InternalPanic
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JmError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
    #[error("internal panic in adapter")]
    InternalPanic,
}

impl IsRetryable for JmError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Unavailable)
    }
}
impl FromPanic for JmError {
    fn from_panic() -> Self {
        Self::InternalPanic
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
    #[error("internal panic in adapter")]
    InternalPanic,
}

impl IsRetryable for VmError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Unavailable)
    }
}
impl FromPanic for VmError {
    fn from_panic() -> Self {
        Self::InternalPanic
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PostCallError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
    #[error("internal panic in adapter")]
    InternalPanic,
}

impl IsRetryable for PostCallError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Unavailable)
    }
}
impl FromPanic for PostCallError {
    fn from_panic() -> Self {
        Self::InternalPanic
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AnalyticsError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("unavailable")]
    Unavailable,
    #[error("internal panic in adapter")]
    InternalPanic,
}

impl IsRetryable for AnalyticsError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::Unavailable)
    }
}
impl FromPanic for AnalyticsError {
    fn from_panic() -> Self {
        Self::InternalPanic
    }
}
