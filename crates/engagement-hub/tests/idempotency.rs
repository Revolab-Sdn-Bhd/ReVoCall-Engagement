//! Integration tests for [`IdempotencyChecker`] and canonical-hash logic.
//!
//! Requires a live Postgres. Run via `just test` or
//! `EH_DATABASE_URL=... cargo test -p engagement-hub`.

use std::time::Duration;

use engagement_hub::idempotency::{
    IdempotencyChecker, IdempotencyResult, PayloadHash, StartEngagementFields,
    canonical_hash_start_engagement, canonical_json_start_engagement,
};
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

fn sample_fields(org_id: &str) -> StartEngagementFields {
    StartEngagementFields {
        org_id: org_id.to_string(),
        channel: "1".into(),
        mode: "1".into(),
        journey_version: "v1.0.0".into(),
        contact_kind: "2".into(),
        contact_id: None,
        contact_phone: Some("+60126013446".into()),
        batch_id: None,
        voice_profile_id: None,
        test_mode: false,
        language: Some("en-MY".into()),
    }
}

/// Insert a minimal engagement row with the given org_id, request_id, and
/// payload_hash so we can test the idempotency lookup path.
async fn seed_engagement(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    request_id: Uuid,
    payload_hash: &PayloadHash,
    status: i16,
) -> Uuid {
    let engagement_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO engagements (
            engagement_id, organization_id, request_id, payload_hash,
            channel, mode, journey_id, journey_version,
            contact_kind, contact_phone_e164,
            created_by_kind, created_by_id, status
        ) VALUES (
            $1, $2, $3, $4,
            1, 1, 'journey-idem-test', 'v1.0.0',
            2, '+60126013446',
            2, 'svc-test', $5
        )"#,
    )
    .bind(engagement_id)
    .bind(org_id)
    .bind(request_id)
    .bind(payload_hash.as_bytes())
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
    engagement_id
}

// ---------------------------------------------------------------------------
// Unit tests for canonical JSON (no DB needed)
// ---------------------------------------------------------------------------

#[test]
fn canonical_json_known_output_is_stable() {
    let fields = StartEngagementFields {
        org_id: "org-abc".into(),
        channel: "1".into(),
        mode: "1".into(),
        journey_version: "v1".into(),
        contact_kind: "2".into(),
        contact_id: None,
        contact_phone: Some("+60126013446".into()),
        batch_id: None,
        voice_profile_id: None,
        test_mode: false,
        language: None,
    };

    let json = canonical_json_start_engagement(&fields);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let obj = parsed.as_object().unwrap();

    assert_eq!(obj["channel"].as_str().unwrap(), "1");
    assert_eq!(obj["contact_phone"].as_str().unwrap(), "+60126013446");
    assert_eq!(obj["mode"].as_str().unwrap(), "1");
    assert_eq!(obj["org_id"].as_str().unwrap(), "org-abc");
    assert!(!obj["test_mode"].as_bool().unwrap());
    assert!(
        !obj.contains_key("batch_id"),
        "absent field must not appear"
    );
    assert!(
        !obj.contains_key("language"),
        "absent field must not appear"
    );
    assert!(
        !obj.contains_key("metadata"),
        "excluded field must not appear"
    );
    assert!(
        !obj.contains_key("display_name"),
        "excluded field must not appear"
    );
}

#[test]
fn hash_for_known_input_is_sha256_of_canonical_json() {
    let fields = StartEngagementFields {
        org_id: "revolab-test-org".into(),
        channel: "1".into(),
        mode: "1".into(),
        journey_version: "v1.0.0".into(),
        contact_kind: "2".into(),
        contact_id: None,
        contact_phone: Some("+60126013446".into()),
        batch_id: None,
        voice_profile_id: None,
        test_mode: false,
        language: Some("en-MY".into()),
    };

    let hash = canonical_hash_start_engagement(&fields);
    let hex = hash.to_hex();

    // Independently compute expected hash
    let json = canonical_json_start_engagement(&fields);
    let expected_hex = {
        use sha2::{Digest, Sha256};
        let d = Sha256::digest(json.as_bytes());
        d.iter().map(|b| format!("{b:02x}")).collect::<String>()
    };

    assert_eq!(
        hex, expected_hex,
        "SHA-256 hex must match independently computed value"
    );
    assert_eq!(hex.len(), 64, "SHA-256 hex must be 64 chars");
}

#[test]
fn fields_with_all_optionals_hash_differs_from_none() {
    let base = sample_fields("org-x");
    let mut with_batch = sample_fields("org-x");
    with_batch.batch_id = Some("batch-001".into());

    assert_ne!(
        canonical_hash_start_engagement(&base),
        canonical_hash_start_engagement(&with_batch),
        "adding batch_id must change the hash"
    );
}

// ---------------------------------------------------------------------------
// Integration tests — IdempotencyChecker with real DB
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_returns_new_when_no_row_exists() {
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();
    let org_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let fields = sample_fields(&org_id.to_string());
    let hash = canonical_hash_start_engagement(&fields);

    let result = checker
        .check(&pool, org_id, request_id, &hash)
        .await
        .unwrap();
    assert_eq!(result, IdempotencyResult::New);
}

#[tokio::test]
async fn check_returns_replay_when_hash_matches_active_engagement() {
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();
    let org_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let fields = sample_fields(&org_id.to_string());
    let hash = canonical_hash_start_engagement(&fields);

    // status=1 (CREATED, non-terminal)
    let seeded_eid = seed_engagement(&pool, org_id, request_id, &hash, 1).await;

    let result = checker
        .check(&pool, org_id, request_id, &hash)
        .await
        .unwrap();

    match result {
        IdempotencyResult::Replay {
            engagement_id,
            is_terminal,
        } => {
            assert_eq!(
                engagement_id, seeded_eid,
                "must return the seeded engagement_id"
            );
            assert!(!is_terminal, "status=1 (CREATED) is not terminal");
        }
        other => panic!("expected Replay, got {:?}", other),
    }
}

#[tokio::test]
async fn check_returns_replay_terminal_for_terminal_status() {
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();
    let org_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let fields = sample_fields(&org_id.to_string());
    let hash = canonical_hash_start_engagement(&fields);

    // status=4 (SUCCESS, terminal)
    seed_engagement(&pool, org_id, request_id, &hash, 4).await;

    let result = checker
        .check(&pool, org_id, request_id, &hash)
        .await
        .unwrap();

    match result {
        IdempotencyResult::Replay { is_terminal, .. } => {
            assert!(is_terminal, "status=4 (SUCCESS) must be terminal");
        }
        other => panic!("expected Replay(terminal), got {:?}", other),
    }
}

#[tokio::test]
async fn check_returns_conflict_when_hash_differs() {
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();
    let org_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();

    // Seed with hash A (channel=1)
    let fields_a = sample_fields(&org_id.to_string());
    let hash_a = canonical_hash_start_engagement(&fields_a);
    seed_engagement(&pool, org_id, request_id, &hash_a, 1).await;

    // Check with hash B (channel=2 — different)
    let mut fields_b = sample_fields(&org_id.to_string());
    fields_b.channel = "2".into();
    let hash_b = canonical_hash_start_engagement(&fields_b);

    let result = checker
        .check(&pool, org_id, request_id, &hash_b)
        .await
        .unwrap();
    assert_eq!(
        result,
        IdempotencyResult::Conflict,
        "differing hash must return Conflict (REQUEST_ID_CONFLICT)"
    );
}

#[tokio::test]
async fn check_returns_new_for_different_org_same_request_id() {
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();

    let org_a = Uuid::new_v4();
    let org_b = Uuid::new_v4(); // different org
    let shared_request_id = Uuid::new_v4(); // same request_id

    let fields_a = sample_fields(&org_a.to_string());
    let hash_a = canonical_hash_start_engagement(&fields_a);

    // Seed for org_a
    seed_engagement(&pool, org_a, shared_request_id, &hash_a, 1).await;

    // Check for org_b with same request_id — must be New (different org scope)
    let fields_b = sample_fields(&org_b.to_string());
    let hash_b = canonical_hash_start_engagement(&fields_b);
    let result = checker
        .check(&pool, org_b, shared_request_id, &hash_b)
        .await
        .unwrap();

    assert_eq!(
        result,
        IdempotencyResult::New,
        "different org_id must give New, not Replay"
    );
}

#[tokio::test]
async fn replay_on_terminal_engagement_returns_is_terminal_true() {
    // AIP-155: re-sending the same request after engagement reaches SUCCESS
    // must return the existing engagement (Replay with is_terminal=true).
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();
    let org_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let fields = sample_fields(&org_id.to_string());
    let hash = canonical_hash_start_engagement(&fields);

    let seeded_eid = seed_engagement(&pool, org_id, request_id, &hash, 4 /* SUCCESS */).await;
    let result = checker
        .check(&pool, org_id, request_id, &hash)
        .await
        .unwrap();

    match result {
        IdempotencyResult::Replay {
            engagement_id,
            is_terminal,
        } => {
            assert_eq!(engagement_id, seeded_eid);
            assert!(is_terminal, "SUCCESS engagement must be terminal");
        }
        other => panic!("expected Replay on terminal, got {:?}", other),
    }
}

#[tokio::test]
async fn replay_on_cancelled_engagement_returns_terminal() {
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();
    let org_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let fields = sample_fields(&org_id.to_string());
    let hash = canonical_hash_start_engagement(&fields);

    seed_engagement(&pool, org_id, request_id, &hash, 5 /* CANCELLED */).await;
    let result = checker
        .check(&pool, org_id, request_id, &hash)
        .await
        .unwrap();

    match result {
        IdempotencyResult::Replay { is_terminal, .. } => {
            assert!(is_terminal, "CANCELLED must be terminal");
        }
        other => panic!("expected Replay(terminal) for CANCELLED, got {:?}", other),
    }
}

#[tokio::test]
async fn replay_on_failed_engagement_returns_terminal() {
    let pool = make_pool().await;
    let checker = IdempotencyChecker::new();
    let org_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let fields = sample_fields(&org_id.to_string());
    let hash = canonical_hash_start_engagement(&fields);

    seed_engagement(&pool, org_id, request_id, &hash, 6 /* FAILED */).await;
    let result = checker
        .check(&pool, org_id, request_id, &hash)
        .await
        .unwrap();

    match result {
        IdempotencyResult::Replay { is_terminal, .. } => {
            assert!(is_terminal, "FAILED must be terminal");
        }
        other => panic!("expected Replay(terminal) for FAILED, got {:?}", other),
    }
}
