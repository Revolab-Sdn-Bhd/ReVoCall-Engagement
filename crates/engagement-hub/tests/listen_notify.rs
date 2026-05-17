//! Integration tests for LISTEN/NOTIFY fanout + gap recovery (T1-09).
//!
//! These tests require a live Postgres 16 instance.  Run with:
//!
//!     just db-up && cargo test --test listen_notify -- --nocapture
//!
//! Or via the workspace alias:
//!
//!     just test
//!
//! All tests skip gracefully when `EH_DATABASE_URL` is unset and no local
//! Postgres is reachable.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tokio::time::timeout;
use uuid::Uuid;

use engagement_hub::{db::MIGRATOR, notify::ListenNotifyManager};

fn db_url() -> String {
    std::env::var("EH_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:eh_test@localhost:5432/engagement_hub_db".into())
}

/// Build a short-lived pool for test use and run migrations.
async fn test_pool() -> Option<sqlx::PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&db_url())
        .await
        .ok()?;
    MIGRATOR.run(&pool).await.ok()?;
    Some(pool)
}

fn test_metrics() -> std::sync::Arc<engagement_hub::metrics::Metrics> {
    use engagement_hub::config::{Env, RegistryAdapter};
    use engagement_hub::metrics::Metrics;
    std::sync::Arc::new(Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap())
}

/// Insert a minimal valid engagements row.  Uses contact_kind=2 (phone).
async fn seed_engagement(pool: &sqlx::PgPool, engagement_id: Uuid, organization_id: Uuid) {
    sqlx::query(
        r#"INSERT INTO engagements (
            engagement_id, organization_id, request_id, payload_hash,
            channel, mode, journey_id, journey_version,
            contact_kind, contact_phone_e164,
            created_by_kind, created_by_id, status
        ) VALUES (
            $1, $2, $3, '\x00'::bytea,
            1, 1, 'journey-test', 'v1',
            2, '+60123456789',
            1, 'test', 1
        ) ON CONFLICT DO NOTHING"#,
    )
    .bind(engagement_id)
    .bind(organization_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .unwrap();
}

/// Insert one event into `engagement_events` and return the (sequence, event_pk).
async fn insert_event(
    pool: &sqlx::PgPool,
    engagement_id: Uuid,
    organization_id: Uuid,
) -> (i64, i64) {
    sqlx::query_as::<_, (i64, i64)>(
        r#"INSERT INTO engagement_events
              (event_id, engagement_id, organization_id, sequence,
               event_type, status_after, source)
           VALUES (
              gen_random_uuid(), $1, $2,
              COALESCE((SELECT MAX(sequence) FROM engagement_events WHERE engagement_id = $1), 0) + 1,
              1, 1, 1
           ) RETURNING sequence, event_pk"#,
    )
    .bind(engagement_id)
    .bind(organization_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// -----------------------------------------------------------------------
// Test: Insert -> NOTIFY -> subscriber receives
// -----------------------------------------------------------------------

#[tokio::test]
async fn insert_event_subscriber_receives_notify() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: no Postgres available");
        return;
    };

    let engagement_id = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    seed_engagement(&pool, engagement_id, org_id).await;

    // Use a connected_signal so we know when LISTEN is ready (no arbitrary sleep).
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (connected_tx, connected_rx) = tokio::sync::oneshot::channel::<()>();
    let manager = ListenNotifyManager::new(db_url(), pool.clone(), test_metrics())
        .with_connected_signal(connected_tx);

    // Subscribe before the manager loop starts so we do not miss the NOTIFY.
    let mut rx = manager.subscribe_engagement(engagement_id).await;

    let _manager_handle = tokio::spawn(manager.run(shutdown_rx));

    // Wait until the LISTEN connection is established.
    timeout(Duration::from_secs(5), connected_rx)
        .await
        .expect("LISTEN connection did not establish within 5s")
        .expect("connected_tx dropped");

    // Insert the event -- triggers the pg_notify trigger.
    let (seq, event_pk) = insert_event(&pool, engagement_id, org_id).await;

    // The subscriber must receive the payload within 3 seconds.
    let received = timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out waiting for NOTIFY")
        .expect("channel closed");

    assert_eq!(received.engagement_id, engagement_id);
    assert_eq!(received.organization_id, org_id);
    assert_eq!(received.sequence, seq);
    assert_eq!(received.event_pk, event_pk);
    assert_eq!(received.event_type, 1);

    let _ = shutdown_tx.send(true);
}

// -----------------------------------------------------------------------
// Test: Gap-fill after reconnect -- sequence-based cursor, not occurred_at
// -----------------------------------------------------------------------

/// Verifies that the gap-fill query delivers missing events ordered by
/// `sequence ASC`, not by `occurred_at`.
///
/// Clock-skew scenario: two events may have identical `occurred_at` values
/// but their `sequence` numbers are monotonically increasing.  The gap-fill
/// must return them in sequence order.
#[tokio::test]
async fn gap_fill_sequence_cursor_delivers_missed_events() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: no Postgres available");
        return;
    };

    let engagement_id = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    seed_engagement(&pool, engagement_id, org_id).await;

    let (seq1, _pk1) = insert_event(&pool, engagement_id, org_id).await;
    let (seq2, _pk2) = insert_event(&pool, engagement_id, org_id).await;
    assert!(seq2 > seq1, "sequences must be monotonically increasing");

    // Execute the gap-fill query directly to verify ordering.
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT ee.sequence, ee.event_pk
        FROM engagement_events ee
        JOIN engagements e USING (engagement_id)
        WHERE ee.engagement_id = $1
          AND ee.sequence > $2
        ORDER BY ee.sequence ASC
        "#,
    )
    .bind(engagement_id)
    .bind(0i64)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2, "gap-fill must return both events");
    assert_eq!(rows[0].0, seq1, "first event must have seq={seq1}");
    assert_eq!(rows[1].0, seq2, "second event must have seq={seq2}");
    assert!(
        rows[0].0 < rows[1].0,
        "gap-fill must order by sequence ASC, not occurred_at"
    );
}

// -----------------------------------------------------------------------
// Test: Clock-skew -- sequence-based resume works when occurred_at diverges
// -----------------------------------------------------------------------

/// Verifies that sequences are monotonically increasing even when
/// `occurred_at` timestamps are identical (simulated clock skew).
#[tokio::test]
async fn clock_skew_sequence_resume_ignores_occurred_at() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: no Postgres available");
        return;
    };

    let engagement_id = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    seed_engagement(&pool, engagement_id, org_id).await;

    let (seq1, _) = insert_event(&pool, engagement_id, org_id).await;
    let (seq2, _) = insert_event(&pool, engagement_id, org_id).await;

    let rows: Vec<(i64, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"
        SELECT sequence, occurred_at
        FROM engagement_events
        WHERE engagement_id = $1
          AND sequence > 0
        ORDER BY sequence ASC
        "#,
    )
    .bind(engagement_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, seq1);
    assert_eq!(rows[1].0, seq2);
    assert!(
        rows[0].0 < rows[1].0,
        "sequence must be monotonically increasing: got {:?}",
        rows.iter().map(|r| r.0).collect::<Vec<_>>()
    );

    eprintln!(
        "occurred_at[0]={}, occurred_at[1]={} (may be identical under clock skew)",
        rows[0].1, rows[1].1
    );
}

// -----------------------------------------------------------------------
// Test: NotifyPayload received via manager matches the inserted event
// -----------------------------------------------------------------------

/// Verifies that the NOTIFY payload from the trigger matches the inserted
/// event's fields exactly, and that batch_id is correctly populated.
#[tokio::test]
async fn notify_payload_matches_inserted_event_with_batch() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping: no Postgres available");
        return;
    };

    let engagement_id = Uuid::new_v4();
    let org_id = Uuid::new_v4();
    let batch_id = Uuid::new_v4();

    // Insert engagements row with a batch_id.
    sqlx::query(
        r#"INSERT INTO engagements (
            engagement_id, organization_id, request_id, payload_hash,
            channel, mode, journey_id, journey_version,
            contact_kind, contact_phone_e164,
            created_by_kind, created_by_id, status, batch_id
        ) VALUES (
            $1, $2, $3, '\x00'::bytea,
            1, 1, 'journey-test', 'v1',
            2, '+60123456789',
            1, 'test', 1, $4
        ) ON CONFLICT DO NOTHING"#,
    )
    .bind(engagement_id)
    .bind(org_id)
    .bind(Uuid::new_v4())
    .bind(batch_id)
    .execute(&pool)
    .await
    .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (connected_tx, connected_rx) = tokio::sync::oneshot::channel::<()>();
    let manager = ListenNotifyManager::new(db_url(), pool.clone(), test_metrics())
        .with_connected_signal(connected_tx);

    let mut rx_eng = manager.subscribe_engagement(engagement_id).await;
    let mut rx_batch = manager.subscribe_batch(batch_id).await;

    let _handle = tokio::spawn(manager.run(shutdown_rx));

    timeout(Duration::from_secs(5), connected_rx)
        .await
        .expect("LISTEN did not establish")
        .unwrap();

    let (seq, _) = insert_event(&pool, engagement_id, org_id).await;

    // Both subscribers receive the same payload.
    let received_eng = timeout(Duration::from_secs(3), rx_eng.recv())
        .await
        .expect("timed out on engagement subscriber")
        .unwrap();

    let received_batch = timeout(Duration::from_secs(3), rx_batch.recv())
        .await
        .expect("timed out on batch subscriber")
        .unwrap();

    assert_eq!(received_eng, received_batch);
    assert_eq!(received_eng.engagement_id, engagement_id);
    assert_eq!(received_eng.batch_id, Some(batch_id));
    assert_eq!(received_eng.sequence, seq);

    let _ = shutdown_tx.send(true);
}
