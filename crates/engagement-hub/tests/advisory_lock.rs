use std::time::Duration;

use sqlx::{Acquire, postgres::PgPoolOptions};
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("EH_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:eh_test@localhost:5432/engagement_hub_db".into())
}

/// Insert a minimal valid engagements row. Uses contact_kind=2 (phone) so the
/// engagements_contact_check constraint is satisfied via contact_phone_e164.
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
        )"#,
    )
    .bind(engagement_id)
    .bind(organization_id)
    .bind(Uuid::new_v4()) // request_id
    .execute(pool)
    .await
    .unwrap();
}

async fn allocate_and_insert_event(
    pool: &sqlx::PgPool,
    engagement_id: Uuid,
    organization_id: Uuid,
) -> i64 {
    let mut conn = pool.acquire().await.unwrap();
    let mut tx = conn.begin().await.unwrap();

    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
        .bind(engagement_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();

    let row: (i64,) = sqlx::query_as(
        r#"INSERT INTO engagement_events
              (event_id, engagement_id, organization_id, sequence,
               event_type, status_after, source)
           VALUES (
              gen_random_uuid(), $1, $2,
              COALESCE((SELECT MAX(sequence) FROM engagement_events WHERE engagement_id = $1), 0) + 1,
              1, 1, 1
           ) RETURNING sequence"#,
    )
    .bind(engagement_id)
    .bind(organization_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();
    row.0
}

#[tokio::test]
async fn two_writers_get_distinct_sequences() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&db_url())
        .await
        .unwrap();

    engagement_hub::db::MIGRATOR.run(&pool).await.unwrap();

    for _ in 0..5 {
        let engagement_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        seed_engagement(&pool, engagement_id, organization_id).await;

        let pool_a = pool.clone();
        let pool_b = pool.clone();
        let (a, b) = tokio::join!(
            tokio::spawn(async move {
                allocate_and_insert_event(&pool_a, engagement_id, organization_id).await
            }),
            tokio::spawn(async move {
                allocate_and_insert_event(&pool_b, engagement_id, organization_id).await
            }),
        );
        let a = a.unwrap();
        let b = b.unwrap();

        let mut seqs = vec![a, b];
        seqs.sort();
        assert_eq!(seqs, vec![1, 2], "sequences must be 1 then 2; got {seqs:?}");
    }
}
