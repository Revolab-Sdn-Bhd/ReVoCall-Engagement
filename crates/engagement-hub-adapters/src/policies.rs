use std::{
    future::Future,
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use futures::FutureExt;
use rand::Rng;

use engagement_hub_ports::error::{FromPanic, IsRetryable};

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
}

// ---------------------------------------------------------------------------
// Core retry + panic-safety combinator
// ---------------------------------------------------------------------------

/// Retries `f` up to `config.max_attempts` on retryable errors, with full-jitter
/// exponential backoff. Panics inside `f` are caught and returned as `E::from_panic()`.
/// Retry counts are recorded to `metrics` if `Some`.
pub async fn with_retry<F, Fut, T, E>(
    config: RetryConfig,
    target: &str,
    metrics: Option<&AdapterMetrics>,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
    E: IsRetryable + FromPanic + Send + 'static,
    T: Send + 'static,
{
    debug_assert!(
        config.max_attempts > 0,
        "RetryConfig::max_attempts must be > 0"
    );
    let mut backoff = config.initial_backoff;
    for attempt in 0..config.max_attempts {
        let result = AssertUnwindSafe(f())
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(E::from_panic()));

        match &result {
            Err(e) if e.is_retryable() && attempt + 1 < config.max_attempts => {
                if let Some(m) = metrics {
                    m.retries_total
                        .with_label_values(&[target, &(attempt + 1).to_string()])
                        .inc();
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
        let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
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
        let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
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
        let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
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
        let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
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
        let r: Result<i32, E> = with_retry(no_sleep_config(1), "t", None, || async move {
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
        let _: Result<i32, E> = with_retry(no_sleep_config(3), "reg", Some(&m), || {
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
}
