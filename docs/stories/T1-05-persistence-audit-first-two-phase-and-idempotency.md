# T1-05: Persistence primitives — audit-first two-phase + idempotency canonical hash

**Issue:** #12 | **Branch:** feat/12-persistence-primitives | **Date:** 2026-05-17

## Brainstorm

### Problem

RevoCall is sold to banks. Every committed engagement must have a matching audit row
from the very first database commit forward — no window where an engagement exists
without an audit record. This story implements two primitives that all future stories
depend on:

1. **AuditWriter** — the audit-first two-phase pattern: phase-1 inserts a `PENDING`
   audit row atomically with the engagement row (same tx); phase-2 updates it with
   the final outcome when orchestration completes (or the reconciler picks it up on
   crash).

2. **Idempotency key checker** — RFC 8785 canonical JSON hash stored per engagement
   so that replayed `StartEngagement` calls with the same `request_id` detect
   payload drift (`REQUEST_ID_CONFLICT`) or return current state (match).

Both are consumed by T1-06 (orchestrator), T1-07 (saga compensation), T1-08
(reconciler), and every control/query RPC in T1-10/T1-11.

### Options considered

**A. Audit-first two-phase, synchronous in-band writes (chosen)**

Insert audit row in the same tx as the engagement row. Phase-2 is an UPDATE on
completion. On crash, a `PENDING` row is left and the reconciler sweeps it. No async
buffer, no queue, no drop policy. Audit failure on tx-1 = RPC returns INTERNAL
immediately. Compliance over availability.

**B. Async audit buffer / fire-and-forget**

Audit write goes to a queue or background task; engagement tx commits first.
Rejected: creates a window where an engagement exists without an audit row. For banks
this is a compliance violation — if we can't audit, we don't act (PRD §9).

**C. Separate audit service**

Audit writes go to a dedicated service. Rejected: adds network hop in the critical
path, makes tx-1 atomicity impossible without distributed transactions. Same
compliance gap as option B.

### Decision

Design pre-determined per PRD §9. All three design decisions are locked by the
compliance requirement:

- **Audit-first two-phase (not async buffer)** — compliance over availability: if
  the audit row can't be committed, the engagement row must not be committed either.
- **RFC 8785 canonical JSON for idempotency hash** — deterministic, cross-language,
  platform-stable. Implemented via `olpc-cjson` crate (no extra network calls).
- **Synchronous in-band audit writes** — the PENDING-finalization reconciler covers
  crash recovery; no async infra needed.
- **`sha2` crate for SHA-256** — already in the Rust ecosystem; no custom crypto.

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `AuditWriter` (two-phase audit row management) and
`IdempotencyChecker` (RFC 8785 canonical hash + replay detection) as library modules
in `crates/engagement-hub`, with unit tests that do not require a live database.

**Architecture:** `AuditWriter` owns all `INSERT`/`UPDATE` logic against
`engagement_audit`; it takes a `sqlx::PgPool` and the `Metrics` handle. Phase-1 uses
a `sqlx::Transaction<Postgres>` passed in from the caller (atomicity with engagement
row); phase-2 uses the pool directly with a 5-attempt retry loop.
`IdempotencyChecker` is pure Rust — no DB dependency in the core logic — computing
`sha256(canonical_json(idempotency_fields))` using `olpc-cjson` + `sha2`. The DB
lookup/insert is a standalone free function that operates on a `PgPool`.

**Tech Stack:** Rust 2024, sqlx 0.8, sha2 0.10, olpc-cjson 1.x, serde_json 1.x,
prometheus 0.13, chrono 0.4, uuid 1.x, tokio 1.x.

### Design decisions

- `AuditWriter` phase-1 takes `&mut sqlx::Transaction<'_, Postgres>` — the caller
  owns the transaction; `AuditWriter` just piggybacks onto it.
- Phase-2 retry uses `tokio::time::sleep` with exponential backoff (50ms, 100ms,
  200ms, 400ms, 800ms) and logs a warning on each retry.
- `IdempotencyFields` is a flat `serde::Serialize` struct whose JSON output is
  passed through RFC 8785 canonicalization; `display_name`, `request_id`, and
  `metadata` are absent from the struct.
- Hash is stored as `BYTEA` (32 bytes) in `engagements.payload_hash`; comparison is
  done in Rust after fetching the row, not in SQL.
- `AuditOutcome` is a Rust enum with explicit `i16` discriminants matching the SQL
  schema: `Pending = 0`, `Success = 1`, `ClientError = 2`, `ServerError = 3`.
- No `[POC]` tasks — `olpc-cjson` is a published crate with documented RFC 8785
  compliance; `sha2` is a well-known RustCrypto crate. Both are straight-line
  additions to `Cargo.toml`.

### File map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `crates/engagement-hub/src/audit.rs` | `AuditWriter` struct + phase-1/phase-2 methods |
| Create | `crates/engagement-hub/src/idempotency.rs` | `IdempotencyFields`, canonical hash, DB check |
| Modify | `crates/engagement-hub/src/lib.rs` | `pub mod audit; pub mod idempotency;` |
| Modify | `crates/engagement-hub/Cargo.toml` | Add `sha2`, `olpc-cjson` dependencies |
| Modify | `Cargo.toml` (workspace) | Add `sha2`, `olpc-cjson` to `[workspace.dependencies]` |

### Task 1: Add crate dependencies

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/engagement-hub/Cargo.toml`

- [ ] **Step 1: Add sha2 + olpc-cjson to workspace dependencies**

Add under `[workspace.dependencies]` in `Cargo.toml`:

```toml
sha2 = "0.10"
olpc-cjson = "0.1"
```

- [ ] **Step 2: Reference from engagement-hub crate**

Add under `[dependencies]` in `crates/engagement-hub/Cargo.toml`:

```toml
sha2.workspace = true
olpc-cjson.workspace = true
```

- [ ] **Step 3: Verify compile**

```bash
cargo build -p engagement-hub 2>&1 | tail -5
```

Expected: exit 0, no errors about unknown packages.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/engagement-hub/Cargo.toml
git commit -m "chore: add sha2 + olpc-cjson deps for T1-05 (#12)"
```

---

### Task 2: AuditWriter module (TDD)

**Files:**

- Create: `crates/engagement-hub/src/audit.rs`

#### Red: write failing tests first

- [ ] **Step 1: Write the test module (audit.rs skeleton + #[cfg(test)])**

Create `crates/engagement-hub/src/audit.rs` with:
- `AuditOutcome` enum (`Pending=0`, `Success=1`, `ClientError=2`, `ServerError=3`)
- `PrincipalKind` enum (`Service=1`, `User=2`)
- `AuditRow` struct with all fields matching `engagement_audit` table
- `AuditWriter` struct (holds `PgPool` + `Arc<Metrics>`)
- `AuditWriter::phase1` signature: takes `&mut Transaction<'_, Postgres>` + `AuditRow`
  → `Result<Uuid>` (returns `audit_id`)
- `AuditWriter::phase2` signature: takes `audit_id: Uuid`, `outcome: AuditOutcome`,
  `error_code: Option<String>` → `Result<()>` with 5-attempt retry
- Test module (no DB): unit tests for `AuditOutcome` discriminants and
  `AuditRow` construction roundtrip

- [ ] **Step 2: Confirm tests compile but the logic is stubbed**

```bash
cargo test -p engagement-hub audit -- --nocapture 2>&1 | tail -20
```

Expected: tests run (may fail or pass on stubs — the key check is compile success).

#### Green: implement AuditWriter

- [ ] **Step 3: Implement `AuditWriter::phase1`**

```rust
/// Insert an audit row with outcome=PENDING into the passed transaction.
/// Called in the same tx as the engagement INSERT (tx-1).
pub async fn phase1(
    &self,
    tx: &mut Transaction<'_, Postgres>,
    row: &AuditRow,
) -> Result<Uuid, AuditError> {
    let audit_id = Uuid::new_v4();
    let timer = self.metrics.audit_insert_duration_seconds.start_timer();
    let result = sqlx::query!(
        r#"
        INSERT INTO engagement_audit (
            audit_id, occurred_at, organization_id,
            acting_principal_kind, acting_principal_id, acting_user_id, acting_via,
            rpc_name, engagement_id, request_id,
            outcome, error_code, request_summary, trace_id
        ) VALUES (
            $1, now(), $2,
            $3, $4, $5, $6,
            $7, $8, $9,
            0, NULL, $10, $11
        )
        "#,
        audit_id,
        row.organization_id,
        row.acting_principal_kind as i16,
        &row.acting_principal_id,
        row.acting_user_id,
        &row.acting_via,
        &row.rpc_name,
        row.engagement_id,
        row.request_id,
        row.request_summary,
        row.trace_id.as_deref(),
    )
    .execute(&mut **tx)
    .await;
    timer.observe_duration();
    result.map_err(|e| {
        self.metrics.audit_insert_failures_total.inc();
        AuditError::Insert(e)
    })?;
    Ok(audit_id)
}
```

- [ ] **Step 4: Implement `AuditWriter::phase2` with 5-attempt retry**

```rust
pub async fn phase2(
    &self,
    audit_id: Uuid,
    outcome: AuditOutcome,
    error_code: Option<String>,
) -> Result<(), AuditError> {
    let delays_ms = [50u64, 100, 200, 400, 800];
    let mut last_err = None;
    for (attempt, &delay_ms) in delays_ms.iter().enumerate() {
        let res = sqlx::query!(
            r#"
            UPDATE engagement_audit
               SET outcome      = $1,
                   error_code   = $2,
                   finalized_at = now()
             WHERE audit_id = $3
            "#,
            outcome as i16,
            error_code.as_deref(),
            audit_id,
        )
        .execute(&self.pool)
        .await;
        match res {
            Ok(_) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    error = %e,
                    "audit phase-2 UPDATE failed; retrying"
                );
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }
    Err(AuditError::Phase2Exhausted(last_err.unwrap()))
}
```

- [ ] **Step 5: Run unit tests (should pass)**

```bash
cargo test -p engagement-hub audit -- --nocapture 2>&1 | tail -20
```

Expected: all audit unit tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/engagement-hub/src/audit.rs
git commit -m "feat: AuditWriter phase-1 + phase-2 with retry (#12)"
```

---

### Task 3: Idempotency module (TDD)

**Files:**

- Create: `crates/engagement-hub/src/idempotency.rs`

#### Red: write failing tests first

- [ ] **Step 1: Write test stubs**

Create `crates/engagement-hub/src/idempotency.rs` with:
- `IdempotencyFields` struct (serializable, no `request_id`/`metadata`/`display_name`)
- `canonical_hash(fields: &IdempotencyFields) -> [u8; 32]` — pure function
- `IdempotencyResult` enum: `Fresh(PayloadHash)`, `Duplicate`, `Conflict`
- Unit tests (no DB) asserting:
  - Same fields → same hash
  - Differ by `org_id` → different hash
  - Fields with different key-ordering in JSON → same hash (RFC 8785 key sort)
  - Serialization round-trip of `IdempotencyFields`

- [ ] **Step 2: Confirm tests compile**

```bash
cargo test -p engagement-hub idempotency -- --nocapture 2>&1 | tail -20
```

#### Green: implement canonical hash

- [ ] **Step 3: Implement `canonical_hash`**

```rust
use olpc_cjson::CanonicalFormatter;
use sha2::{Digest, Sha256};

pub fn canonical_hash(fields: &IdempotencyFields) -> [u8; 32] {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, CanonicalFormatter::new());
    fields.serialize(&mut ser).expect("IdempotencyFields is always serializable");
    Sha256::digest(&buf).into()
}
```

- [ ] **Step 4: Implement `check_idempotency` DB function**

```rust
/// Idempotency flow per PRD §9:
/// 1. SELECT engagement WHERE (org, request_id)
/// 2a. Found + hash match → return Duplicate (caller returns current state)
/// 2b. Found + hash mismatch → return Conflict
/// 3. Not found → INSERT ON CONFLICT DO NOTHING; if 0 rows returned, loop (concurrent dup)
pub async fn check_idempotency(
    pool: &PgPool,
    org_id: Uuid,
    request_id: Uuid,
    hash: [u8; 32],
    new_engagement_id: Uuid,
    // ... other INSERT fields
) -> Result<IdempotencyResult, IdempotencyError>
```

- [ ] **Step 5: Run unit tests**

```bash
cargo test -p engagement-hub idempotency -- --nocapture 2>&1 | tail -20
```

Expected: all hash unit tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/engagement-hub/src/idempotency.rs
git commit -m "feat: IdempotencyChecker RFC 8785 canonical hash (#12)"
```

---

### Task 4: Wire modules into lib.rs

**Files:**

- Modify: `crates/engagement-hub/src/lib.rs`

- [ ] **Step 1: Add module declarations**

```rust
pub mod audit;
pub mod idempotency;
```

- [ ] **Step 2: Verify full workspace build**

```bash
cargo build --workspace 2>&1 | tail -10
```

Expected: exit 0.

- [ ] **Step 3: Commit**

```bash
git add crates/engagement-hub/src/lib.rs
git commit -m "chore: expose audit + idempotency modules (#12)"
```

---

### Task 5: Verify audit indices exist in schema

**Files:**

- Read: `migrations/20260515000000_initial_schema.up.sql`

- [ ] **Step 1: Confirm all four required indices are present**

Required per acceptance criteria:
- `engagement_audit_pending_idx` — `WHERE outcome = 0`
- `engagement_audit_org_time_idx` — `(organization_id, occurred_at DESC)`
- `engagement_audit_engagement_idx` — `(engagement_id) WHERE engagement_id IS NOT NULL`
- `engagement_audit_trace_idx` — `(trace_id)`

All four are present in the initial schema migration (verified during brainstorm).
No migration changes needed.

- [ ] **Step 2: Commit no-op confirmation note**

This task is verification-only; no code changes.

---

### Task 6: Metrics registration guard

**Files:**

- Read: `crates/engagement-hub/src/metrics.rs`

- [ ] **Step 1: Verify required metrics are already registered**

Per acceptance criteria:
- `engagementhub_audit_insert_duration_seconds` (Histogram) — already present
- `engagementhub_audit_insert_failures_total` (Counter) — already present

Both were implemented in T3-02. No changes needed.

- [ ] **Step 2: Run metrics unit tests as guard**

```bash
cargo test -p engagement-hub metrics -- --nocapture 2>&1 | tail -20
```

Expected: all pass.

---

### Deferred

- Proto-level `idempotency_fields` comment validation CI test — tracked in the
  proto stories (T2-01 / T2-02); CI test requires proto files to be finalized.
- `StopEngagement` / `CancelEngagement` idempotency field lists — those RPCs are
  implemented in T1-10/T1-11; the `IdempotencyFields` enum will be extended there.
- Read-RPC audit (synchronous single-phase, no PENDING) — T1-10/T1-11.
- Watch-stream audit row lifecycle (open/close) — T1-09.
- Reconciler audit-finalization sweep — T1-08.
