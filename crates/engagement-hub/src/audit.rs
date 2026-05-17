//! Audit-first two-phase writer (PRD §9).
//!
//! ## Bank-grade guarantee
//!
//! Phase-1 inserts a PENDING audit row *within* the caller's transaction (tx-1),
//! alongside the engagement row. If tx-1 rolls back (DB down, constraint violation,
//! etc.) the audit row is never written — nothing happened, nothing is audited.
//! If tx-1 commits but the process then crashes, the PENDING row is swept by the
//! reconciler (T1-09). This ensures "if we can't audit, we don't act."
//!
//! ## Phase-2 finalization
//!
//! After orchestration resolves (success or failure), the caller finalises the audit
//! row by calling [`AuditWriter::phase2_finalize`]. Phase-2 retries up to 5 times
//! with a short back-off; if all attempts fail it logs an error and increments the
//! `engagementhub_audit_insert_failures_total` counter (alert threshold: >0).
//!
//! ## Synchronous / in-band
//!
//! There is **no** async buffer and **no** drop policy. Phase-1 failure → RPC returns
//! INTERNAL immediately. Phase-2 failure → logged + metric; reconciler will eventually
//! sweep the PENDING row.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::Value as JsonValue;
use sqlx::{PgConnection, PgPool};
use tracing::{error, warn};
use uuid::Uuid;

use crate::metrics::Metrics;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Opaque identifier for a single audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditId(Uuid);

impl AuditId {
    /// Create a fresh random [`AuditId`].
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Return the inner [`Uuid`].
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Consume and return the inner [`Uuid`].
    pub fn into_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for AuditId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AuditId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Audit outcome values (PRD §9).
///
/// The discriminant values match the `outcome` SMALLINT stored in the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum AuditOutcome {
    /// In-flight; reconciler sweeps after 30 s.
    Pending = 0,
    /// RPC completed successfully.
    Success = 1,
    /// 4xx-class client error.
    ClientError = 2,
    /// 5xx-class server error.
    ServerError = 3,
}

impl AuditOutcome {
    fn as_i16(self) -> i16 {
        self as i16
    }
}

/// Kind of principal that issued the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum PrincipalKind {
    /// Unknown / unset.
    Unknown = 0,
    /// Human user authenticated via IdP.
    User = 1,
    /// Machine-to-machine service account.
    ServiceAccount = 2,
    /// Internal system component (e.g. reconciler).
    System = 3,
}

impl PrincipalKind {
    fn as_i16(self) -> i16 {
        self as i16
    }
}

/// All data required for Phase-1 insertion.
#[derive(Debug, Clone)]
pub struct AuditRow {
    /// Pre-allocated ID (caller generates so it can be returned immediately).
    pub audit_id: AuditId,
    /// Organisation that owns the request.
    pub organization_id: Option<Uuid>,
    /// Kind of principal (user, service-account, system).
    pub acting_principal_kind: PrincipalKind,
    /// Stable identifier for the principal (sub claim, service-account ID, …).
    pub acting_principal_id: String,
    /// Human user on whose behalf a service-account acts (may be None).
    pub acting_user_id: Option<Uuid>,
    /// "via" label, e.g. `"grpc/StartEngagement"`.
    pub acting_via: String,
    /// Proto RPC name, e.g. `"StartEngagement"`.
    pub rpc_name: String,
    /// Pre-allocated engagement ID written atomically with the audit row.
    pub engagement_id: Option<Uuid>,
    /// Request ID from the RPC header.
    pub request_id: Option<Uuid>,
    /// Redacted JSON summary of the request (PII-stripped by caller).
    pub request_summary: Option<JsonValue>,
    /// W3C trace-id from the active OpenTelemetry span.
    pub trace_id: Option<String>,
}

// ---------------------------------------------------------------------------
// AuditWriter
// ---------------------------------------------------------------------------

/// Stateless writer handle. Cheap to clone; backed by a shared [`Arc<Metrics>`].
pub struct AuditWriter {
    metrics: Arc<Metrics>,
}

impl AuditWriter {
    /// Construct a writer.
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self { metrics }
    }

    /// **Phase 1** — insert an audit row with `outcome = PENDING`.
    ///
    /// This method MUST be called within the caller's existing transaction
    /// (`conn` is `&mut PgConnection`, not a pool). The atomicity guarantee is:
    /// if the caller's transaction rolls back, this insert is also rolled back.
    ///
    /// On any DB error this method returns `Err`; the caller MUST propagate that
    /// error and return `INTERNAL` to the RPC client.
    pub async fn phase1_insert(
        &self,
        conn: &mut PgConnection,
        row: &AuditRow,
    ) -> Result<AuditId, sqlx::Error> {
        let t0 = Instant::now();

        let result = sqlx::query(
            r#"
            INSERT INTO engagement_audit (
                audit_id,
                occurred_at,
                finalized_at,
                organization_id,
                acting_principal_kind,
                acting_principal_id,
                acting_user_id,
                acting_via,
                rpc_name,
                engagement_id,
                request_id,
                outcome,
                error_code,
                request_summary,
                trace_id,
                archived_at
            ) VALUES (
                $1, now(), NULL, $2, $3, $4, $5, $6, $7, $8, $9,
                0,   -- AUDIT_OUTCOME_PENDING
                NULL, $10, $11, NULL
            )
            "#,
        )
        .bind(row.audit_id.as_uuid())
        .bind(row.organization_id)
        .bind(row.acting_principal_kind.as_i16())
        .bind(&row.acting_principal_id)
        .bind(row.acting_user_id)
        .bind(&row.acting_via)
        .bind(&row.rpc_name)
        .bind(row.engagement_id)
        .bind(row.request_id)
        .bind(&row.request_summary)
        .bind(&row.trace_id)
        .execute(conn)
        .await;

        let elapsed = t0.elapsed().as_secs_f64();
        self.metrics.audit_insert_duration_seconds.observe(elapsed);

        match result {
            Ok(_) => Ok(row.audit_id),
            Err(e) => {
                self.metrics.audit_insert_failures_total.inc();
                error!(
                    audit_id = %row.audit_id,
                    error = %e,
                    "phase-1 audit insert failed — returning INTERNAL"
                );
                Err(e)
            }
        }
    }

    /// **Phase 2** — update outcome + `finalized_at` on an existing PENDING row.
    ///
    /// Retries up to 5 times with exponential back-off (50 ms → 100 ms → 200 ms …).
    /// If all attempts fail, logs the error and increments the failure counter.
    /// Callers do **not** need to propagate this error; the reconciler will sweep
    /// the PENDING row if phase-2 ultimately fails.
    pub async fn phase2_finalize(
        &self,
        pool: &PgPool,
        audit_id: AuditId,
        outcome: AuditOutcome,
        error_code: Option<&str>,
    ) {
        const MAX_ATTEMPTS: u32 = 5;
        let mut delay = Duration::from_millis(50);

        for attempt in 1..=MAX_ATTEMPTS {
            let result = self
                .phase2_attempt(pool, audit_id, outcome, error_code)
                .await;

            match result {
                Ok(()) => return,
                Err(e) => {
                    if attempt < MAX_ATTEMPTS {
                        warn!(
                            audit_id = %audit_id,
                            attempt,
                            error = %e,
                            "phase-2 audit finalize failed, retrying"
                        );
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                    } else {
                        error!(
                            audit_id = %audit_id,
                            attempt,
                            error = %e,
                            "phase-2 audit finalize failed after all attempts — PENDING row will be swept"
                        );
                        self.metrics.audit_insert_failures_total.inc();
                    }
                }
            }
        }
    }

    /// Single phase-2 attempt (not retried internally).
    async fn phase2_attempt(
        &self,
        pool: &PgPool,
        audit_id: AuditId,
        outcome: AuditOutcome,
        error_code: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let now = Utc::now();

        sqlx::query(
            r#"
            UPDATE engagement_audit
            SET    outcome      = $1,
                   finalized_at = $2,
                   error_code   = $3
            WHERE  audit_id = $4
            "#,
        )
        .bind(outcome.as_i16())
        .bind(now)
        .bind(error_code)
        .bind(audit_id.as_uuid())
        .execute(pool)
        .await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Env, RegistryAdapter};

    fn make_writer() -> AuditWriter {
        let metrics = Arc::new(Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap());
        AuditWriter::new(metrics)
    }

    fn sample_row() -> AuditRow {
        AuditRow {
            audit_id: AuditId::new(),
            organization_id: Some(Uuid::new_v4()),
            acting_principal_kind: PrincipalKind::ServiceAccount,
            acting_principal_id: "svc-test".into(),
            acting_user_id: None,
            acting_via: "grpc/StartEngagement".into(),
            rpc_name: "StartEngagement".into(),
            engagement_id: Some(Uuid::new_v4()),
            request_id: Some(Uuid::new_v4()),
            request_summary: Some(serde_json::json!({"channel": 1})),
            trace_id: Some("abc123".into()),
        }
    }

    #[test]
    fn audit_id_display_is_uuid_string() {
        let id = AuditId::new();
        let s = id.to_string();
        // UUID string is 36 chars with 4 hyphens
        assert_eq!(s.len(), 36);
        assert_eq!(s.chars().filter(|&c| c == '-').count(), 4);
    }

    #[test]
    fn audit_outcome_discriminants() {
        assert_eq!(AuditOutcome::Pending.as_i16(), 0);
        assert_eq!(AuditOutcome::Success.as_i16(), 1);
        assert_eq!(AuditOutcome::ClientError.as_i16(), 2);
        assert_eq!(AuditOutcome::ServerError.as_i16(), 3);
    }

    #[test]
    fn principal_kind_discriminants() {
        assert_eq!(PrincipalKind::Unknown.as_i16(), 0);
        assert_eq!(PrincipalKind::User.as_i16(), 1);
        assert_eq!(PrincipalKind::ServiceAccount.as_i16(), 2);
        assert_eq!(PrincipalKind::System.as_i16(), 3);
    }

    #[test]
    fn sample_row_builds() {
        let row = sample_row();
        assert_eq!(row.rpc_name, "StartEngagement");
    }

    #[test]
    fn writer_constructs() {
        let w = make_writer();
        drop(w);
    }
}
