//! Fake port implementations for use in unit tests.
//!
//! Each fake uses a per-method response queue with injectable outcomes and is
//! thread-safe via `Arc<Mutex<...>>`.

pub mod analytics;
pub mod journey_manager;
pub mod post_call;
pub mod registry;
pub mod voice_manager;

pub use analytics::FakeAnalyticsPort;
pub use journey_manager::FakeJourneyManagerPort;
pub use post_call::FakePostCallPort;
pub use registry::FakeRegistryPort;
pub use voice_manager::FakeVoiceManagerPort;

/// Injectable response type for fake port implementations.
///
/// Push outcomes onto a fake's queue before calling the method under test.
#[derive(Debug)]
pub enum Outcome<T> {
    /// Return `Ok(val)`.
    Success(T),
    /// Return `Err(XxxError::Transient(msg))`.
    Transient(String),
    /// Return `Err(XxxError::Permanent(msg))`.
    Permanent(String),
    /// Return `Err(XxxError::Unavailable)`.
    Unavailable,
    /// Call `panic!` inside the method.
    Panic,
}
