//! Integration tests for [`AuditWriter`].
//!
//! Requires a live Postgres (set `EH_DATABASE_URL` or rely on the default).
//! Run via `just test` or `EH_DATABASE_URL=... cargo test -p engagement-hub`.

use std::sync::Arc;
use std::time::Duration;

use engagement_hub::audit::{AuditId, AuditOutcome, AuditRow, AuditWriter, PrincipalKind};
use engagement_hub::config::{Env, RegistryAdapter};
use engagement_hub::metrics::Metrics;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("EH_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:eh_test@localhost:5432/engagement_hub_db".into())
}

async fn make_pool() -> sqlx::PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&db_url())
        .await
        .expect("Failed to connect to Postgres — is the DB up?");
    engagement_hub::db::MIGRATOR
        .run(&pool)
        .await
        .expect("migration failed");
    pool
}

fn make_writer() -> AuditWriter {
    let metrics = Arc::new(Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap());
    AuditWriter::new(metrics)
}

fn sample_row_without_engagement() -> AuditRow {
    AuditRow {
        audit_id: AuditId::new(),
        organization_id: Some(Uuid::new_v4()),
        acting_principal_kind: PrincipalKind::ServiceAccount,
        acting_principal_id: "test-svc".into(),
        acting_user_id: None,
        acting_via: "grpc/TestRPC".into(),
        rpc_name: "TestRPC".into(),
        engagement_id: None, // no FK needed
        request_id: Some(Uuid::new_v4()),
        request_summary: Some(serde_json::json!({"channel": 1, "mode": 1})),
        trace_id: Some("aabbcc".into()),
    }
}

// ---------------------------------------------------------------------------
// Phase-1 happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn phase1_insert_writes_pending_row() {
    let pool = make_pool().await;
    let writer = make_writer();
    let row = sample_row_without_engagement();
    let audit_id = row.audit_id;

    let mut tx = pool.begin().await.unwrap();
    let returned_id = writer.phase1_insert(&mut tx, &row).await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(returned_id, audit_id);

    // Verify the row is present with outcome=0 (PENDING)
    let (outcome,): (i16,) =
        sqlx::query_as("SELECT outcome FROM engagement_audit WHERE audit_id = $1")
            .bind(audit_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(outcome, 0, "phase-1 row must have outcome=PENDING (0)");
}

#[tokio::test]
async fn phase1_row_has_null_finalized_at() {
    let pool = make_pool().await;
    let writer = make_writer();
    let row = sample_row_without_engagement();
    let audit_id = row.audit_id;

    let mut tx = pool.begin().await.unwrap();
    writer.phase1_insert(&mut tx, &row).await.unwrap();
    tx.commit().await.unwrap();

    let (finalized_at,): (Option<chrono::DateTime<chrono::Utc>>,) =
        sqlx::query_as("SELECT finalized_at FROM engagement_audit WHERE audit_id = $1")
            .bind(audit_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        finalized_at.is_none(),
        "phase-1 row must have finalized_at = NULL"
    );
}

#[tokio::test]
async fn phase1_stores_request_summary() {
    let pool = make_pool().await;
    let writer = make_writer();
    let mut row = sample_row_without_engagement();
    row.request_summary = Some(serde_json::json!({"channel": 2, "test_mode": true}));
    let audit_id = row.audit_id;

    let mut tx = pool.begin().await.unwrap();
    writer.phase1_insert(&mut tx, &row).await.unwrap();
    tx.commit().await.unwrap();

    let (summary,): (serde_json::Value,) =
        sqlx::query_as("SELECT request_summary FROM engagement_audit WHERE audit_id = $1")
            .bind(audit_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(summary["channel"], 2);
    assert_eq!(summary["test_mode"], true);
}

// ---------------------------------------------------------------------------
// Phase-1 atomicity — rollback scenario
// ---------------------------------------------------------------------------

#[tokio::test]
async fn phase1_rollback_leaves_no_row() {
    let pool = make_pool().await;
    let writer = make_writer();
    let row = sample_row_without_engagement();
    let audit_id = row.audit_id;

    {
        let mut tx = pool.begin().await.unwrap();
        writer.phase1_insert(&mut tx, &row).await.unwrap();
        // Deliberately rollback instead of commit — simulates tx-1 failure
        tx.rollback().await.unwrap();
    }

    let maybe_row: Option<(i16,)> =
        sqlx::query_as("SELECT outcome FROM engagement_audit WHERE audit_id = $1")
            .bind(audit_id.as_uuid())
            .fetch_optional(&pool)
            .await
            .unwrap();

    assert!(
        maybe_row.is_none(),
        "rolled-back tx must leave no audit row — atomicity guarantee violated"
    );
}

// ---------------------------------------------------------------------------
// Phase-2 finalize — success path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn phase2_finalizes_pending_to_success() {
    let pool = make_pool().await;
    let writer = make_writer();
    let row = sample_row_without_engagement();
    let audit_id = row.audit_id;

    // Phase 1
    let mut tx = pool.begin().await.unwrap();
    writer.phase1_insert(&mut tx, &row).await.unwrap();
    tx.commit().await.unwrap();

    // Phase 2
    writer
        .phase2_finalize(&pool, audit_id, AuditOutcome::Success, None)
        .await;

    let (outcome, finalized_at): (i16, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT outcome, finalized_at FROM engagement_audit WHERE audit_id = $1")
            .bind(audit_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(outcome, 1, "outcome must be SUCCESS (1) after phase-2");
    assert!(
        finalized_at.is_some(),
        "finalized_at must be set after phase-2"
    );
}

#[tokio::test]
async fn phase2_finalizes_to_client_error_with_code() {
    let pool = make_pool().await;
    let writer = make_writer();
    let row = sample_row_without_engagement();
    let audit_id = row.audit_id;

    let mut tx = pool.begin().await.unwrap();
    writer.phase1_insert(&mut tx, &row).await.unwrap();
    tx.commit().await.unwrap();

    writer
        .phase2_finalize(
            &pool,
            audit_id,
            AuditOutcome::ClientError,
            Some("INVALID_ARGUMENT"),
        )
        .await;

    let (outcome, error_code): (i16, Option<String>) =
        sqlx::query_as("SELECT outcome, error_code FROM engagement_audit WHERE audit_id = $1")
            .bind(audit_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(outcome, 2, "outcome must be CLIENT_ERROR (2)");
    assert_eq!(
        error_code.as_deref(),
        Some("INVALID_ARGUMENT"),
        "error_code must be stored"
    );
}

#[tokio::test]
async fn phase2_finalizes_to_server_error() {
    let pool = make_pool().await;
    let writer = make_writer();
    let row = sample_row_without_engagement();
    let audit_id = row.audit_id;

    let mut tx = pool.begin().await.unwrap();
    writer.phase1_insert(&mut tx, &row).await.unwrap();
    tx.commit().await.unwrap();

    writer
        .phase2_finalize(&pool, audit_id, AuditOutcome::ServerError, Some("INTERNAL"))
        .await;

    let (outcome,): (i16,) =
        sqlx::query_as("SELECT outcome FROM engagement_audit WHERE audit_id = $1")
            .bind(audit_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(outcome, 3, "outcome must be SERVER_ERROR (3)");
}

// ---------------------------------------------------------------------------
// Phase-2 no-op on unknown audit_id (robustness, not an error)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn phase2_nonexistent_audit_id_does_not_panic() {
    let pool = make_pool().await;
    let writer = make_writer();
    let fake_id = AuditId::new();

    // Should complete without panic (UPDATE … WHERE … affects 0 rows, which is fine).
    writer
        .phase2_finalize(&pool, fake_id, AuditOutcome::Success, None)
        .await;
}

// ---------------------------------------------------------------------------
// Metrics are emitted on phase-1 insert
// ---------------------------------------------------------------------------

#[tokio::test]
async fn phase1_increments_duration_histogram() {
    let pool = make_pool().await;
    let metrics = Arc::new(Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap());
    let writer = AuditWriter::new(metrics.clone());
    let row = sample_row_without_engagement();

    let mut tx = pool.begin().await.unwrap();
    writer.phase1_insert(&mut tx, &row).await.unwrap();
    tx.commit().await.unwrap();

    // The histogram count must be at least 1 after one successful insert.
    let text = metrics.gather_text().unwrap();
    assert!(
        text.contains("engagementhub_audit_insert_duration_seconds_count 1"),
        "audit histogram count must be 1 after one insert\n{text}"
    );
}

// ---------------------------------------------------------------------------
// DB-down simulation — acquiring from a dead pool fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pg_down_acquire_fails() {
    // Use an unreachable port to simulate PG-down.
    // connect_lazy succeeds (no connection yet); acquire will fail.
    let bad_pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy("postgres://postgres:eh_test@127.0.0.1:19999/nonexistent")
        .unwrap();

    // Verify the pool cannot serve a connection — simulates PG-down condition.
    let result = bad_pool.acquire().await;
    assert!(
        result.is_err(),
        "acquiring from a dead pool must fail (simulates PG-down)"
    );
}
