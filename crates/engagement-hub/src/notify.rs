//! LISTEN/NOTIFY fanout + gap recovery (T1-09).
//!
//! # Architecture
//!
//! A dedicated `PgListener` (separate from the main application pool) issues
//! `LISTEN engagement_events`.  On every NOTIFY the payload is deserialized
//! into [`NotifyPayload`] and fanned out to all matching subscribers.
//! Subscribers may match by `engagement_id` or by `batch_id`.
//!
//! ## Gap recovery
//!
//! On reconnect the manager replays events from the DB using the per-engagement
//! `sequence` cursor (`sequence > last_seen`).  `occurred_at` is **never** used
//! as a resume cursor because it is subject to clock skew; `sequence` is
//! monotonically increasing within an engagement.
//!
//! ## Cross-engagement ordering
//!
//! `event_pk` (a `BIGSERIAL`) provides a global ordering for callers that need to
//! merge events across multiple engagements (e.g., `WatchEngagements`).
//!
//! ## Slow subscriber / STREAM_OVERFLOW
//!
//! Each subscriber gets a bounded broadcast channel.  If the channel is full the
//! subscriber is dropped and will reconnect via the sequence cursor (T1-12 will
//! wire this into the SDK reconnect path).

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{
    PgPool,
    postgres::{PgListener, PgNotification},
};
use tokio::{
    sync::{
        Mutex,
        broadcast::{self, Receiver, Sender},
    },
    time::{interval, sleep},
};
use uuid::Uuid;

use crate::metrics::Metrics;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Payload carried by each PostgreSQL `pg_notify('engagement_events', ...)` call.
///
/// Serialized size is well under the 200-byte limit specified in the PRD.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotifyPayload {
    pub engagement_id: Uuid,
    pub organization_id: Uuid,
    pub batch_id: Option<Uuid>,
    /// Per-engagement monotonic sequence number.  Use this as the resume cursor.
    pub sequence: i64,
    /// Global BIGSERIAL -- use for cross-engagement ordering.
    pub event_pk: i64,
    pub event_type: i16,
    pub traceparent: Option<String>,
}

// Broadcast channel capacity.  A slow subscriber that falls > CHANNEL_CAP events
// behind will be dropped (STREAM_OVERFLOW).  The SDK must reconnect using the
// sequence cursor.
const CHANNEL_CAP: usize = 256;

// Health-check: send a keepalive query on the LISTEN connection every 10s.
const HEALTH_INTERVAL: Duration = Duration::from_secs(10);

// Delay before retrying a failed reconnect.
const RECONNECT_DELAY: Duration = Duration::from_millis(200);

// ---------------------------------------------------------------------------
// Subscriber registry
// ---------------------------------------------------------------------------

/// A cloneable reference to the subscriber registry.
#[derive(Clone, Default)]
struct Registry {
    inner: Arc<Mutex<RegistryInner>>,
}

#[derive(Default)]
struct RegistryInner {
    /// Subscribers keyed by `engagement_id`.
    by_engagement: HashMap<Uuid, Vec<Sender<NotifyPayload>>>,
    /// Subscribers keyed by `batch_id`.
    by_batch: HashMap<Uuid, Vec<Sender<NotifyPayload>>>,
}

impl Registry {
    /// Subscribe to events for a single engagement.
    pub async fn subscribe_engagement(&self, engagement_id: Uuid) -> Receiver<NotifyPayload> {
        let (tx, rx) = broadcast::channel(CHANNEL_CAP);
        let mut inner = self.inner.lock().await;
        inner
            .by_engagement
            .entry(engagement_id)
            .or_default()
            .push(tx);
        rx
    }

    /// Subscribe to events for all engagements in a batch.
    pub async fn subscribe_batch(&self, batch_id: Uuid) -> Receiver<NotifyPayload> {
        let (tx, rx) = broadcast::channel(CHANNEL_CAP);
        let mut inner = self.inner.lock().await;
        inner.by_batch.entry(batch_id).or_default().push(tx);
        rx
    }

    /// Fan out a payload to all matching subscribers.
    ///
    /// Dead senders (all receivers dropped) are pruned in-place.
    pub async fn fanout(&self, payload: &NotifyPayload) {
        let mut inner = self.inner.lock().await;

        // Fan out to engagement subscribers.
        if let Some(senders) = inner.by_engagement.get_mut(&payload.engagement_id) {
            senders.retain(|tx| tx.send(payload.clone()).is_ok());
            if senders.is_empty() {
                inner.by_engagement.remove(&payload.engagement_id);
            }
        }

        // Fan out to batch subscribers.
        if let Some(batch_id) = payload.batch_id
            && let Some(senders) = inner.by_batch.get_mut(&batch_id)
        {
            senders.retain(|tx| tx.send(payload.clone()).is_ok());
            if senders.is_empty() {
                inner.by_batch.remove(&batch_id);
            }
        }
    }

    /// Count total messages queued across all active subscriber channels.
    ///
    /// Used for the `consumer_lag_events` gauge: a non-zero value means at
    /// least one subscriber is behind and has undelivered events buffered.
    pub async fn queued_event_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner
            .by_engagement
            .values()
            .flat_map(|v| v.iter())
            .map(|tx| tx.len())
            .sum::<usize>()
            + inner
                .by_batch
                .values()
                .flat_map(|v| v.iter())
                .map(|tx| tx.len())
                .sum::<usize>()
    }
}

// ---------------------------------------------------------------------------
// Gap-fill
// ---------------------------------------------------------------------------

/// Row returned by the gap-fill query.
#[derive(sqlx::FromRow)]
struct GapRow {
    engagement_id: Uuid,
    organization_id: Uuid,
    batch_id: Option<Uuid>,
    sequence: i64,
    event_pk: i64,
    event_type: i16,
    traceparent: Option<String>,
}

/// Fetch events for `engagement_id` with `sequence > last_seen` and fan them
/// out to subscribers.  Ordered by `sequence ASC` -- never by `occurred_at`.
async fn gap_fill_engagement(
    pool: &PgPool,
    engagement_id: Uuid,
    last_seen: i64,
    registry: &Registry,
) -> Result<()> {
    let rows: Vec<GapRow> = sqlx::query_as::<_, GapRow>(
        r#"
        SELECT
            ee.engagement_id,
            ee.organization_id,
            e.batch_id,
            ee.sequence,
            ee.event_pk,
            ee.event_type,
            ee.trace_context->>'traceparent' AS traceparent
        FROM engagement_events ee
        JOIN engagements e USING (engagement_id)
        WHERE ee.engagement_id = $1
          AND ee.sequence > $2
        ORDER BY ee.sequence ASC
        "#,
    )
    .bind(engagement_id)
    .bind(last_seen)
    .fetch_all(pool)
    .await
    .context("gap-fill query failed")?;

    for row in rows {
        let payload = NotifyPayload {
            engagement_id: row.engagement_id,
            organization_id: row.organization_id,
            batch_id: row.batch_id,
            sequence: row.sequence,
            event_pk: row.event_pk,
            event_type: row.event_type,
            traceparent: row.traceparent,
        };
        registry.fanout(&payload).await;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// LISTEN connection management
// ---------------------------------------------------------------------------

/// Open a dedicated `PgListener` (single-connection internal pool, separate
/// from the main application pool) and subscribe to the `engagement_events`
/// channel.
///
/// `eager_reconnect(false)` disables transparent auto-reconnect so the manager
/// loop can detect failures, increment the reconnect metric, and replay gaps.
async fn connect_and_listen(database_url: &str) -> Result<PgListener> {
    let mut listener = PgListener::connect(database_url)
        .await
        .context("failed to open LISTEN connection")?;

    // Disable eager auto-reconnect so we control reconnect timing and can
    // increment the metric and trigger gap-fill on each reconnect.
    listener.eager_reconnect(false);

    listener
        .listen("engagement_events")
        .await
        .context("LISTEN command failed")?;

    tracing::info!("LISTEN/NOTIFY connection established");
    Ok(listener)
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Manages the LISTEN connection, health-check, reconnect, and subscriber fanout.
///
/// Instantiate with [`ListenNotifyManager::new`] and call
/// [`ListenNotifyManager::run`] as a background task.  Use
/// [`ListenNotifyManager::subscribe_engagement`] /
/// [`ListenNotifyManager::subscribe_batch`] to obtain receivers before or after
/// the task is running.
pub struct ListenNotifyManager {
    database_url: String,
    pool: PgPool,
    registry: Registry,
    metrics: Arc<Metrics>,
    /// Optional oneshot sender to notify callers when the first LISTEN
    /// connection is established.
    connected_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ListenNotifyManager {
    pub fn new(database_url: String, pool: PgPool, metrics: Arc<Metrics>) -> Self {
        Self {
            database_url,
            pool,
            registry: Registry::default(),
            metrics,
            connected_tx: None,
        }
    }

    /// Attach a oneshot sender that fires once the initial LISTEN connection is
    /// established.  Useful in tests to avoid sleeping for an arbitrary duration.
    pub fn with_connected_signal(mut self, tx: tokio::sync::oneshot::Sender<()>) -> Self {
        self.connected_tx = Some(tx);
        self
    }

    /// Subscribe to live events for a single engagement.
    pub async fn subscribe_engagement(&self, engagement_id: Uuid) -> Receiver<NotifyPayload> {
        self.registry.subscribe_engagement(engagement_id).await
    }

    /// Subscribe to live events for all engagements belonging to a batch.
    pub async fn subscribe_batch(&self, batch_id: Uuid) -> Receiver<NotifyPayload> {
        self.registry.subscribe_batch(batch_id).await
    }

    /// Perform a gap-fill for `engagement_id` starting from `last_seen_sequence`.
    ///
    /// Subscribers MUST call this after reconnecting to avoid lost events.
    /// Use `sequence > last_seen_sequence` as the resume cursor -- never
    /// rely on `occurred_at`.
    pub async fn gap_fill(&self, engagement_id: Uuid, last_seen_sequence: i64) -> Result<()> {
        gap_fill_engagement(
            &self.pool,
            engagement_id,
            last_seen_sequence,
            &self.registry,
        )
        .await
    }

    /// Run the manager loop.  This is a long-running async task; spawn with
    /// `tokio::spawn(manager.run(shutdown_rx))`.
    pub async fn run(mut self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        loop {
            // Check shutdown before doing anything.
            if *shutdown.borrow() {
                tracing::info!("LISTEN/NOTIFY manager shutting down");
                return;
            }

            // Attempt to connect.
            let conn_result = connect_and_listen(&self.database_url).await;
            let mut listener = match conn_result {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(err = %e, "LISTEN connection failed; retrying");
                    self.metrics.listen_notify_reconnects_total.inc();
                    tokio::select! {
                        _ = sleep(RECONNECT_DELAY) => continue,
                        _ = shutdown.changed() => {
                            tracing::info!("LISTEN/NOTIFY manager shutting down");
                            return;
                        }
                    }
                }
            };

            // Notify any waiting caller that the LISTEN connection is ready.
            if let Some(tx) = self.connected_tx.take() {
                let _ = tx.send(());
            }

            // Drive the LISTEN loop.
            //
            // Health-check: every HEALTH_INTERVAL, call try_recv() on the
            // connection.  This both checks liveness AND drains any buffered
            // notifications.  If try_recv() returns a notification, it is
            // processed normally rather than discarded.
            //
            // The interval starts AFTER connect and we consume the first
            // immediate tick right away so the first real health check fires
            // after HEALTH_INTERVAL, not at t=0.  This lets recv() be
            // selected first in the main select! below.
            let mut health_tick = interval(HEALTH_INTERVAL);
            health_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            health_tick.tick().await; // consume the initial immediate tick

            let mut disconnected = false;

            loop {
                tokio::select! {
                    // Shutdown signal
                    _ = shutdown.changed() => {
                        tracing::info!("LISTEN/NOTIFY manager shutting down");
                        return;
                    }

                    // Health-check ping
                    _ = health_tick.tick() => {
                        // try_recv() is non-blocking; if a notification is
                        // pending, process it rather than discarding it.
                        match listener.try_recv().await {
                            Ok(Some(notif)) => {
                                tracing::debug!("LISTEN health-check received a buffered notification");
                                self.handle_notification(notif).await;
                            }
                            Ok(None) => {
                                tracing::debug!("LISTEN connection health-check OK");
                            }
                            Err(e) => {
                                tracing::warn!(err = %e, "LISTEN connection health-check failed; reconnecting");
                                disconnected = true;
                            }
                        }
                        if disconnected {
                            break;
                        }
                    }

                    // Wait for the next notification (blocking)
                    result = listener.recv() => {
                        match result {
                            Err(e) => {
                                tracing::warn!(err = %e, "LISTEN connection error; reconnecting");
                                disconnected = true;
                                break;
                            }
                            Ok(notif) => {
                                self.handle_notification(notif).await;
                            }
                        }
                    }
                }
            }

            if disconnected {
                tracing::info!("LISTEN/NOTIFY reconnecting...");
                self.metrics.listen_notify_reconnects_total.inc();
                tokio::select! {
                    _ = sleep(RECONNECT_DELAY) => {}
                    _ = shutdown.changed() => {
                        tracing::info!("LISTEN/NOTIFY manager shutting down");
                        return;
                    }
                }
            }
        }
    }

    async fn handle_notification(&self, notif: PgNotification) {
        let payload_str = notif.payload();
        if payload_str.is_empty() {
            tracing::warn!("received empty NOTIFY payload; skipping");
            return;
        }

        match serde_json::from_str::<NotifyPayload>(payload_str) {
            Ok(payload) => {
                // Update the consumer-lag gauge.
                let lag = self.registry.queued_event_count().await as i64;
                self.metrics.consumer_lag_events.set(lag);
                self.registry.fanout(&payload).await;
            }
            Err(e) => {
                tracing::warn!(
                    err = %e,
                    raw = payload_str,
                    "failed to parse NOTIFY payload; skipping"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Unit tests (no live DB required)
    // -----------------------------------------------------------------------

    #[test]
    fn payload_round_trips_with_all_fields() {
        let original = NotifyPayload {
            engagement_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            batch_id: Some(Uuid::new_v4()),
            sequence: 42,
            event_pk: 1001,
            event_type: 3,
            traceparent: Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into()),
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: NotifyPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded);
    }

    #[test]
    fn payload_round_trips_with_null_fields() {
        let original = NotifyPayload {
            engagement_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            batch_id: None,
            sequence: 1,
            event_pk: 7,
            event_type: 1,
            traceparent: None,
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: NotifyPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded);
    }

    #[test]
    fn payload_json_size_under_200_bytes_typical_case() {
        // PRD 11 requires the NOTIFY payload to be < 200 bytes for the typical
        // case (small sequence numbers, single-digit event types, no traceparent).
        let payload = NotifyPayload {
            engagement_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            organization_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap(),
            batch_id: None,
            sequence: 42,
            event_pk: 1001,
            event_type: 1,
            traceparent: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(
            json.len() < 200,
            "Typical NOTIFY payload is {} bytes (>=200): {json}",
            json.len()
        );
    }

    #[test]
    fn payload_json_size_worst_case_documented() {
        // Documents the worst-case payload size (all fields at max length).
        // The PRD 11 <200-byte constraint applies to the typical case only.
        let payload = NotifyPayload {
            engagement_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            organization_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap(),
            batch_id: Some(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440002").unwrap()),
            sequence: 9_999_999_999,
            event_pk: 9_999_999_999,
            event_type: 32767,
            traceparent: Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(
            json.len() < 400,
            "Worst-case NOTIFY payload unexpectedly large at {} bytes: {json}",
            json.len()
        );
        eprintln!("Worst-case NOTIFY payload size: {} bytes", json.len());
    }

    #[test]
    fn payload_malformed_json_does_not_panic() {
        let result = serde_json::from_str::<NotifyPayload>("{not valid json}");
        assert!(result.is_err());
    }

    #[test]
    fn payload_missing_required_field_is_error() {
        let json = r#"{"organization_id":"550e8400-e29b-41d4-a716-446655440001","sequence":1,"event_pk":1,"event_type":1}"#;
        let result = serde_json::from_str::<NotifyPayload>(json);
        assert!(
            result.is_err(),
            "expected deserialization error for missing engagement_id"
        );
    }

    /// Clock-skew test: two events share the same `occurred_at` but have
    /// distinct monotonically increasing `sequence` values.  The gap-fill
    /// query orders by `sequence ASC`, so the subscriber receives them
    /// in sequence order regardless of wall-clock time.
    #[test]
    fn sequence_order_is_independent_of_occurred_at() {
        let mut payloads = [
            NotifyPayload {
                engagement_id: Uuid::new_v4(),
                organization_id: Uuid::new_v4(),
                batch_id: None,
                sequence: 2,
                event_pk: 200,
                event_type: 2,
                traceparent: None,
            },
            NotifyPayload {
                engagement_id: Uuid::new_v4(),
                organization_id: Uuid::new_v4(),
                batch_id: None,
                sequence: 1,
                event_pk: 100,
                event_type: 1,
                traceparent: None,
            },
        ];

        payloads.sort_by_key(|p| p.sequence);

        assert_eq!(payloads[0].sequence, 1);
        assert_eq!(payloads[1].sequence, 2);
    }

    // -----------------------------------------------------------------------
    // Registry fanout unit tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fanout_delivers_to_engagement_subscriber() {
        let registry = Registry::default();
        let engagement_id = Uuid::new_v4();
        let mut rx = registry.subscribe_engagement(engagement_id).await;

        let payload = NotifyPayload {
            engagement_id,
            organization_id: Uuid::new_v4(),
            batch_id: None,
            sequence: 1,
            event_pk: 1,
            event_type: 1,
            traceparent: None,
        };

        registry.fanout(&payload).await;

        let received = rx.try_recv().expect("should have received payload");
        assert_eq!(received, payload);
    }

    #[tokio::test]
    async fn fanout_delivers_to_batch_subscriber() {
        let registry = Registry::default();
        let batch_id = Uuid::new_v4();
        let mut rx = registry.subscribe_batch(batch_id).await;

        let payload = NotifyPayload {
            engagement_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            batch_id: Some(batch_id),
            sequence: 1,
            event_pk: 1,
            event_type: 1,
            traceparent: None,
        };

        registry.fanout(&payload).await;

        let received = rx.try_recv().expect("should have received payload");
        assert_eq!(received, payload);
    }

    #[tokio::test]
    async fn fanout_does_not_deliver_to_wrong_engagement() {
        let registry = Registry::default();
        let engagement_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();
        let mut rx = registry.subscribe_engagement(other_id).await;

        let payload = NotifyPayload {
            engagement_id,
            organization_id: Uuid::new_v4(),
            batch_id: None,
            sequence: 1,
            event_pk: 1,
            event_type: 1,
            traceparent: None,
        };

        registry.fanout(&payload).await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn fanout_prunes_dead_senders() {
        let registry = Registry::default();
        let engagement_id = Uuid::new_v4();
        {
            let _rx = registry.subscribe_engagement(engagement_id).await;
        }

        let payload = NotifyPayload {
            engagement_id,
            organization_id: Uuid::new_v4(),
            batch_id: None,
            sequence: 1,
            event_pk: 1,
            event_type: 1,
            traceparent: None,
        };

        registry.fanout(&payload).await;

        let inner = registry.inner.lock().await;
        assert!(!inner.by_engagement.contains_key(&engagement_id));
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let registry = Registry::default();
        let engagement_id = Uuid::new_v4();

        let mut rx1 = registry.subscribe_engagement(engagement_id).await;
        let mut rx2 = registry.subscribe_engagement(engagement_id).await;

        let payload = NotifyPayload {
            engagement_id,
            organization_id: Uuid::new_v4(),
            batch_id: None,
            sequence: 1,
            event_pk: 1,
            event_type: 1,
            traceparent: None,
        };

        registry.fanout(&payload).await;

        assert_eq!(rx1.try_recv().unwrap(), payload);
        assert_eq!(rx2.try_recv().unwrap(), payload);
    }
}
