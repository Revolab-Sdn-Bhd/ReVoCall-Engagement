# T1-05: Persistence primitives — audit-first two-phase + idempotency canonical hash

**Issue:** #12 | **Branch:** feat/12-persistence-primitives | **Date:** 2026-05-17

## Brainstorm

### Problem

PRD §9 mandates bank-grade audit guarantees and AIP-155 idempotency for every mutating
RPC. Without audit-first two-phase, a crashed process between commit and orchestration
leaves no record that a request was attempted. Without canonical-hash idempotency,
duplicate StartEngagement calls can create duplicate engagements.

Design is fully specified by the issue and PRD:

- `AuditWriter` with `phase1_insert` (PENDING row, written atomically in tx-1 alongside
  the engagement row) and `phase2_finalize` (outcome + finalized\_at, 5-attempt retry).
- `IdempotencyChecker` with canonical JSON hash (RFC 8785) + per-RPC allow-list.
- Both metrics (`engagementhub_audit_insert_duration_seconds`,
  `engagementhub_audit_insert_failures_total`) already exist in metrics.rs from T1-01.
- `engagement_audit` table and all four required indices already exist in migration.
- No new migration needed.

### Key decisions

**Decision A — Pure-sqlx, dynamic queries for `AuditWriter`.**
sqlx offline-mode requires `.sqlx/` cache files generated against a live DB. Because
audit.rs and idempotency.rs introduce novel query shapes, using `query!` macros would
require committing regenerated cache files in the same PR, or blocking CI on a live DB
step. We use `sqlx::query()` (dynamic) with typed `.bind()` calls. The schemas are
fixed and well-understood; the risk of runtime mismatch is covered by the integration
tests that run against a real Postgres.

**Decision B — Idempotency in `engagement-hub` crate (not ports).**
`IdempotencyChecker` is a service-layer concern that touches `sqlx::PgPool` directly.
It does not belong in `ports` (which is DB-agnostic) or `adapters` (outbound only).

**Decision C — sha2 + serde\_json + BTreeMap for canonical hash.**
`olpc-cjson` is unmaintained. Manual key-sorting via `BTreeMap` + `serde_json::to_string`
gives deterministic UTF-8 output equivalent to RFC 8785 for the string/UUID/bool values
in the StartEngagement allow-list. `sha2` (SHA-256) from the `sha2` crate is used for
the 32-byte digest, stored as `payload_hash BYTEA` on the engagement row.

**Decision D — `AuditWriter` takes `&mut PgConnection` for phase-1.**
Phase-1 must run *within* the caller's transaction (tx-1). Passing `&mut PgConnection`
lets the caller open the transaction and the writer participate in it without requiring
a full transaction object in the API. Phase-2 takes `&PgPool` and retries with a fresh
connection each attempt.

**Decision E — `IdempotencyChecker::check` fetches all columns in one query.**
The original flow description suggested two queries (first fetch hash+status, then
fetch engagement\_id). A single `SELECT payload_hash, status, engagement_id` avoids
the extra round-trip; results are the same under REPEATABLE READ isolation.

## Implementation plan

### Goal

Implement `AuditWriter` and `IdempotencyChecker` as library modules in
`crates/engagement-hub/src/`, add `sha2` to deps, write comprehensive unit +
integration tests, and commit a story doc.

### Architecture

```text
engagement-hub
├── src/
│   ├── audit.rs          (new) AuditWriter, AuditRow, AuditId, AuditOutcome
│   ├── idempotency.rs    (new) IdempotencyChecker, StartEngagementFields, PayloadHash
│   ├── lib.rs            (mod audit; mod idempotency;)
│   └── ... (unchanged)
tests/
├── audit_writer.rs       (new) integration: phase-1/2 atomicity, PG-down sim
├── idempotency.rs        (new) integration: hash match/mismatch, replay
```

### Tech stack

- `sha2 = "0.10"` — SHA-256 digest
- `sqlx 0.8` — dynamic queries; `PgConnection` for phase-1, `PgPool` for phase-2
- `serde_json` + `std::collections::BTreeMap` — deterministic key-sorted JSON
- `chrono` — `finalized_at` timestamps
- `uuid` — `AuditId`, `EngagementId`, `RequestId`, `OrgId`

### File map

| File | Change |
| ------ | -------- |
| `Cargo.toml` (workspace) | add `sha2 = "0.10"` |
| `crates/engagement-hub/Cargo.toml` | add `sha2.workspace = true` |
| `crates/engagement-hub/src/audit.rs` | new |
| `crates/engagement-hub/src/idempotency.rs` | new |
| `crates/engagement-hub/src/lib.rs` | add `pub mod audit; pub mod idempotency;` |
| `crates/engagement-hub/tests/audit_writer.rs` | new integration tests |
| `crates/engagement-hub/tests/idempotency.rs` | new integration tests |
| `docs/stories/T1-05-persistence-primitives.md` | this file |

### Tasks

- [x] Task 1: Add `sha2` dep; create `audit.rs` with `AuditRow`, `AuditId`,
  `AuditOutcome`, `AuditWriter::phase1_insert`, `AuditWriter::phase2_finalize`
- [x] Task 2: Create `idempotency.rs` with `StartEngagementFields`, `canonical_hash`,
  `IdempotencyChecker`, `IdempotencyResult`, `PayloadHash`
- [x] Task 3: Expose modules in `lib.rs`
- [x] Task 4: Integration tests — `audit_writer.rs` (phase-1/2, atomicity, PG-down)
- [x] Task 5: Integration tests — `idempotency.rs` (hash unit tests, match/mismatch/replay)
- [x] Task 6: CI gate — `cargo fmt`, `cargo clippy`, `markdownlint`

### Deferred

- Hook `AuditWriter` and `IdempotencyChecker` into actual RPC handlers — deferred to
  T1-06 (orchestrator / StartEngagement handler implementation).
- Reconciler sweep of PENDING audit rows — deferred to T1-09.
