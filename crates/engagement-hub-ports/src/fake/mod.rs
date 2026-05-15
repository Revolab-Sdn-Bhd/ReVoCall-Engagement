//! Fake port implementations for use in unit tests.
//!
//! Each fake uses a per-method response queue with injectable outcomes and is
//! thread-safe via `Arc<Mutex<...>>`.

pub mod registry;
pub mod journey_manager;
pub mod voice_manager;
pub mod post_call;
pub mod analytics;

pub use registry::FakeRegistryPort;
pub use journey_manager::FakeJourneyManagerPort;
pub use voice_manager::FakeVoiceManagerPort;
pub use post_call::FakePostCallPort;
pub use analytics::FakeAnalyticsPort;

/// Injectable response type for fake port implementations.
///
/// Push outcomes onto a fake's queue before calling the method under test.
pub enum Outcome<T> {
    /// Return `Ok(val)`.
    Success(T),
    /// Return `Err(XxxError::Transient(msg))`.
    Transient(String),
    /// Return `Err(XxxError::Permanent(msg))`.
    Permanent(String),
    /// Call `panic!` inside the method.
    Panic,
}
