use std::{
    future::Future,
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use futures::FutureExt;
use rand::Rng;

use engagement_hub_ports::error::{FromDeadline, FromPanic, IsRetryable};

use crate::metrics::AdapterMetrics;

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

/// 5 attempts — used for Registry.resolve_snapshot (read-only, cheap to retry).
pub const REGISTRY_RESOLVE_RETRY: RetryConfig = RetryConfig {
    max_attempts: 5,
    initial_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_secs(2),
};

/// 3 attempts — default for PostCall, Analytics, and Registry.get_voice_profile.
pub const DEFAULT_RETRY: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_secs(2),
};

/// 2 attempts — used for write operations whose downstream idempotency comes
/// from a per-call `request_id`. PRD §12: writes idempotent via request_id.
pub const WRITE_RETRY: RetryConfig = RetryConfig {
    max_attempts: 2,
    initial_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_secs(2),
};

/// 5 attempts — used for cleanup operations (`*.stop`/`*.cancel`) that MUST
/// clean up downstream resources. PRD §12 saga compensation budget.
pub const CLEANUP_RETRY: RetryConfig = RetryConfig {
    max_attempts: 5,
    initial_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_secs(2),
};

/// 1 attempt — used for non-idempotent operations like
/// `stop_voice_session(mode=Graceful)`. Per `engagement_hub_ports::types::StopMode`,
/// Graceful is documented as NOT idempotent, so retries are unsafe.
pub const GRACEFUL_STOP_RETRY: RetryConfig = RetryConfig {
    max_attempts: 1,
    initial_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_secs(2),
};

// ---------------------------------------------------------------------------
// Deadline
// ---------------------------------------------------------------------------

const PROPAGATION_MARGIN: Duration = Duration::from_millis(50);
const ADAPTER_FLOOR: Duration = Duration::from_millis(200);

pub struct DeadlineContext {
    deadline: Option<Instant>,
}

impl DeadlineContext {
    pub fn none() -> Self {
        Self { deadline: None }
    }

    /// Compute `min(remaining - 50ms, adapter_default)`.
    pub fn from_remaining(remaining: Duration, adapter_default: Duration) -> Self {
        let budget = remaining
            .saturating_sub(PROPAGATION_MARGIN)
            .min(adapter_default);
        Self {
            deadline: Some(Instant::now() + budget),
        }
    }

    /// True if remaining time is below the 200ms safety floor.
    pub fn is_too_close(&self) -> bool {
        self.deadline
            .is_some_and(|d| d.saturating_duration_since(Instant::now()) < ADAPTER_FLOOR)
    }

    /// Returns the remaining duration until the deadline, or None if no deadline is set.
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline.map(|d| d.saturating_duration_since(Instant::now()))
    }
}

// ---------------------------------------------------------------------------
// Core retry + panic-safety combinator
// ---------------------------------------------------------------------------

/// Retries `f` up to `config.max_attempts` on retryable errors, with full-jitter
/// exponential backoff. Panics inside `f` are caught and returned as `E::from_panic()`.
/// Retry counts are recorded to `metrics` if `Some`.
pub async fn with_retry<F, Fut, T, E>(
    config: RetryConfig,
    deadline: Option<&DeadlineContext>,
    target: &str,
    metrics: Option<&AdapterMetrics>,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
    E: IsRetryable + FromPanic + FromDeadline + Send + 'static,
    T: Send + 'static,
{
    debug_assert!(
        config.max_attempts > 0,
        "RetryConfig::max_attempts must be > 0"
    );
    let mut backoff = config.initial_backoff;
    for attempt in 0..config.max_attempts {
        // Two-stage panic catch: first wrap the synchronous call to f() so a
        // panic in the closure's sync prefix (e.g. proto request construction)
        // is converted to E::from_panic(); then wrap the returned future so
        // panics during polling are also caught.
        let result = match std::panic::catch_unwind(AssertUnwindSafe(&mut f)) {
            Ok(fut) => AssertUnwindSafe(fut)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(E::from_panic())),
            Err(_) => Err(E::from_panic()),
        };

        match &result {
            Err(e) if e.is_retryable() && attempt + 1 < config.max_attempts => {
                if let Some(m) = metrics {
                    m.retries_total
                        .with_label_values(&[target, &(attempt + 1).to_string()])
                        .inc();
                }
                // Deadline gate before next attempt. Per PRD §12: refuse retry if
                // remaining < (next backoff + adapter floor). is_too_close() handles
                // the floor-only case; we add the backoff-aware check here.
                if let Some(d) = deadline {
                    let need = backoff + ADAPTER_FLOOR;
                    if d.remaining().map_or(false, |r| r < need) {
                        if let Some(m) = metrics {
                            m.deadline_exceeded_total.with_label_values(&[target]).inc();
                        }
                        return Err(E::from_deadline());
                    }
                }
                let jitter = rand::thread_rng().gen_range(Duration::ZERO..=backoff);
                tokio::time::sleep(jitter).await;
                backoff = (backoff * 2).min(config.max_backoff);
            }
            _ => return result,
        }
    }
    unreachable!()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    #[derive(Debug, PartialEq, Clone)]
    enum E {
        Transient,
        Permanent,
        Panic,
        Deadline,
    }
    impl IsRetryable for E {
        fn is_retryable(&self) -> bool {
            matches!(self, Self::Transient)
        }
    }
    impl FromPanic for E {
        fn from_panic() -> Self {
            Self::Panic
        }
    }
    impl FromDeadline for E {
        fn from_deadline() -> Self {
            Self::Deadline
        }
    }

    fn no_sleep_config(max: u32) -> RetryConfig {
        RetryConfig {
            max_attempts: max,
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn success_on_first_attempt() {
        let n = Arc::new(AtomicU32::new(0));
        let c = n.clone();
        let r: Result<i32, E> = with_retry(no_sleep_config(3), None, "t", None, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            }
        })
        .await;
        assert_eq!(r, Ok(42));
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_transient_then_succeeds() {
        let n = Arc::new(AtomicU32::new(0));
        let c = n.clone();
        let r: Result<i32, E> = with_retry(no_sleep_config(3), None, "t", None, || {
            let c = c.clone();
            async move {
                let count = c.fetch_add(1, Ordering::SeqCst);
                if count < 2 { Err(E::Transient) } else { Ok(1) }
            }
        })
        .await;
        assert_eq!(r, Ok(1));
        assert_eq!(n.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn no_retry_on_permanent() {
        let n = Arc::new(AtomicU32::new(0));
        let c = n.clone();
        let r: Result<i32, E> = with_retry(no_sleep_config(3), None, "t", None, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(E::Permanent)
            }
        })
        .await;
        assert_eq!(r, Err(E::Permanent));
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn exhausts_all_attempts_on_persistent_transient() {
        let n = Arc::new(AtomicU32::new(0));
        let c = n.clone();
        let r: Result<i32, E> = with_retry(no_sleep_config(3), None, "t", None, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(E::Transient)
            }
        })
        .await;
        assert_eq!(r, Err(E::Transient));
        assert_eq!(n.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn catches_panic_and_returns_from_panic() {
        let r: Result<i32, E> = with_retry(no_sleep_config(1), None, "t", None, || async move {
            panic!("adapter panic")
        })
        .await;
        assert_eq!(r, Err(E::Panic));
    }

    #[tokio::test]
    async fn records_retry_metrics() {
        let m = crate::metrics::AdapterMetrics::for_test();
        let n = Arc::new(AtomicU32::new(0));
        let c = n.clone();
        let _: Result<i32, E> = with_retry(no_sleep_config(3), None, "reg", Some(&m), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(E::Transient)
            }
        })
        .await;
        // 3 attempts → 2 retries recorded
        assert_eq!(m.retries_total.with_label_values(&["reg", "1"]).get(), 1);
        assert_eq!(m.retries_total.with_label_values(&["reg", "2"]).get(), 1);
    }

    #[test]
    fn deadline_too_close_when_remaining_less_than_200ms() {
        let ctx =
            DeadlineContext::from_remaining(Duration::from_millis(100), Duration::from_secs(5));
        assert!(ctx.is_too_close());
    }

    #[test]
    fn deadline_not_too_close_when_remaining_generous() {
        let ctx = DeadlineContext::from_remaining(Duration::from_secs(10), Duration::from_secs(5));
        assert!(!ctx.is_too_close());
    }

    #[test]
    fn deadline_none_never_too_close() {
        assert!(!DeadlineContext::none().is_too_close());
    }

    #[test]
    fn write_retry_is_two_attempts() {
        assert_eq!(WRITE_RETRY.max_attempts, 2);
    }

    #[test]
    fn cleanup_retry_is_five_attempts() {
        assert_eq!(CLEANUP_RETRY.max_attempts, 5);
    }

    #[tokio::test]
    async fn catches_panic_in_synchronous_closure_prefix() {
        // The closure panics BEFORE returning a future. Today's with_retry
        // (pre-T1-04) only wraps the returned future in catch_unwind, so this
        // panic would escape. The fix wraps the call to f() itself.
        let r: Result<i32, E> = with_retry(no_sleep_config(1), None, "t", None, || {
            panic!("sync-prefix panic");
            #[allow(unreachable_code)]
            async move {
                Ok::<i32, E>(0)
            }
        })
        .await;
        assert_eq!(r, Err(E::Panic));
    }

    #[tokio::test]
    async fn deadline_too_close_short_circuits_before_next_attempt() {
        let ctx =
            DeadlineContext::from_remaining(Duration::from_millis(100), Duration::from_secs(5));
        // is_too_close()==true; first attempt should run, but no retry should be attempted.
        let n = Arc::new(AtomicU32::new(0));
        let c = n.clone();
        let r: Result<i32, E> = with_retry(no_sleep_config(3), Some(&ctx), "t", None, || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(E::Transient)
            }
        })
        .await;
        // First attempt ran, deadline check fires before attempt 2.
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert_eq!(r, Err(E::Deadline));
    }

    #[tokio::test]
    async fn deadline_gate_considers_next_backoff() {
        // remaining = 250ms, adapter_floor = 200ms.
        // First attempt sets backoff to 50ms initially; after attempt 1 fails, deadline check sees
        // `need = 50ms + 200ms = 250ms`; remaining is right at boundary or below — fires.
        let ctx = DeadlineContext::from_remaining(
            Duration::from_millis(250),
            Duration::from_secs(5),
        );
        let n = Arc::new(AtomicU32::new(0));
        let c = n.clone();
        let r: Result<i32, E> = with_retry(
            RetryConfig {
                max_attempts: 3,
                initial_backoff: Duration::from_millis(50),
                max_backoff: Duration::from_secs(2),
            },
            Some(&ctx),
            "t",
            None,
            || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err(E::Transient)
                }
            },
        )
        .await;
        // First attempt ran, deadline check (50ms backoff + 200ms floor = 250ms need) fires.
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert_eq!(r, Err(E::Deadline));
    }
}
