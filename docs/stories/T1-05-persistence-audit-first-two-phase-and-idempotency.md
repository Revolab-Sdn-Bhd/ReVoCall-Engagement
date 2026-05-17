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

2. **Idempotency key checker** — sha256 of canonical JSON hash stored per engagement
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

Design pre-determined per PRD §9. The following key decisions were made during
implementation:

- **Audit-first two-phase (not async buffer)** — compliance over availability: if
  the audit row can't be committed, the engagement row must not be committed either.

- **Pure-sqlx dynamic queries for `AuditWriter`** — sqlx offline-mode requires
  `.sqlx/` cache files generated against a live DB. Using `sqlx::query()` (dynamic)
  with typed `.bind()` calls avoids committing cache files while coverage is provided
  by integration tests against a real Postgres.

- **sha2 + serde\_json + BTreeMap for canonical hash** — `olpc-cjson` is
  unmaintained. Manual key-sorting via `BTreeMap` + `serde_json::to_string` gives
  deterministic UTF-8 output equivalent to RFC 8785 for the string/UUID/bool values
  in the StartEngagement allow-list. Simpler, no extra dep, fully testable.

- **`AuditWriter` takes `&mut PgConnection` for phase-1** — phase-1 must run within
  the caller's transaction (tx-1). Passing `&mut PgConnection` lets the caller open
  the transaction and the writer participate in it. Phase-2 takes `&PgPool` and
  retries with a fresh connection each attempt.

- **`IdempotencyChecker::check` fetches all columns in one query** — single
  `SELECT payload_hash, status, engagement_id` avoids an extra round-trip; results
  are equivalent under REPEATABLE READ isolation.

- **Idempotency in `engagement-hub` crate (not ports)** — `IdempotencyChecker` is a
  service-layer concern touching `sqlx::PgPool` directly; it does not belong in
  `ports` (DB-agnostic) or `adapters` (outbound only).

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `AuditWriter` and `IdempotencyChecker` as library modules in
`crates/engagement-hub/src/`, write comprehensive unit + integration tests.

**Architecture:** `AuditWriter` owns all `INSERT`/`UPDATE` logic against
`engagement_audit`; phase-1 uses a `&mut PgConnection` (caller's tx), phase-2 uses
`&PgPool` with 5-attempt exponential-backoff retry. `IdempotencyChecker` is a thin
wrapper around a single DB read; canonical hash is `sha256(BTreeMap-sorted JSON)`.

**Tech Stack:** Rust 2024, sqlx 0.8 (dynamic queries), sha2 0.10, serde_json +
BTreeMap (RFC 8785 equivalent), prometheus 0.13, chrono 0.4, uuid 1.x, tokio 1.x.

### Design decisions

- `AuditWriter::phase1_insert` takes `&mut PgConnection` — participates in caller's tx
- `AuditWriter::phase2_finalize` takes `&PgPool` — retries 5x with exponential backoff
- `AuditOutcome` is `#[repr(i16)]` with discriminants matching SQL schema (0/1/2/3)
- `PrincipalKind` is `#[repr(i16)]` matching SQL schema (0/1/2/3)
- `PayloadHash` is a newtype over `[u8; 32]` stored as `BYTEA` in DB
- Canonical JSON: `BTreeMap<&str, serde_json::Value>` serialised via serde_json (keys sorted)
- Optional fields absent from `StartEngagementFields` are omitted from JSON (not null)
- No `[POC]` tasks — sha2 and BTreeMap+serde_json are well-established patterns

### File map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `crates/engagement-hub/src/audit.rs` | `AuditWriter`, `AuditRow`, `AuditId`, `AuditOutcome`, `PrincipalKind` |
| Create | `crates/engagement-hub/src/idempotency.rs` | `IdempotencyChecker`, `StartEngagementFields`, `PayloadHash`, `IdempotencyResult` |
| Modify | `crates/engagement-hub/src/lib.rs` | `pub mod audit; pub mod idempotency;` |
| Modify | `Cargo.toml` (workspace) | Add `sha2 = "0.10"` |
| Modify | `crates/engagement-hub/Cargo.toml` | Add `sha2.workspace = true` |
| Create | `crates/engagement-hub/tests/audit_writer.rs` | Integration tests for phase-1/2, atomicity, PG-down |
| Create | `crates/engagement-hub/tests/idempotency.rs` | Integration tests for hash match/mismatch/replay |

### Task 1: Add sha2 dependency

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/engagement-hub/Cargo.toml`

- [x] **Step 1:** Add `sha2 = "0.10"` to workspace `[workspace.dependencies]`
- [x] **Step 2:** Add `sha2.workspace = true` to `crates/engagement-hub/Cargo.toml`
- [x] **Step 3:** Verify `cargo build -p engagement-hub` exits 0
- [x] **Step 4:** Commit `chore: add sha2 + olpc-cjson deps for T1-05 (#12)`

---

### Task 2: AuditWriter module (TDD)

**Files:**

- Create: `crates/engagement-hub/src/audit.rs`

- [x] **Step 1:** Write struct definitions + unit test stubs (red)
- [x] **Step 2:** Implement `phase1_insert` (uses `&mut PgConnection`, timed by histogram)
- [x] **Step 3:** Implement `phase2_finalize` (5 retries, exponential backoff 50ms→800ms)
- [x] **Step 4:** Verify unit tests pass (`cargo test -p engagement-hub audit`)
- [x] **Step 5:** Commit `feat: AuditWriter phase-1 + phase-2 with retry (#12)`

---

### Task 3: Idempotency module (TDD)

**Files:**

- Create: `crates/engagement-hub/src/idempotency.rs`

- [x] **Step 1:** Write `StartEngagementFields`, `PayloadHash`, `IdempotencyResult` stubs + unit tests (red)
- [x] **Step 2:** Implement `canonical_json_start_engagement` (BTreeMap key sort)
- [x] **Step 3:** Implement `canonical_hash_start_engagement` (sha256 of canonical JSON)
- [x] **Step 4:** Implement `IdempotencyChecker::check` (single-query DB lookup)
- [x] **Step 5:** Verify unit tests pass (`cargo test -p engagement-hub idempotency`)
- [x] **Step 6:** Commit `feat: IdempotencyChecker RFC 8785 canonical hash (#12)`

---

### Task 4: Integration tests

**Files:**

- Create: `crates/engagement-hub/tests/audit_writer.rs`
- Create: `crates/engagement-hub/tests/idempotency.rs`

- [x] **Step 1:** Write `audit_writer.rs` — phase-1 PENDING row, atomicity (rollback), phase-2 finalize, PG-down simulation
- [x] **Step 2:** Write `idempotency.rs` — canonical JSON unit tests, hash match/mismatch/replay against real DB
- [x] **Step 3:** Commit `test: integration tests for audit + idempotency (#12)`

---

### Task 5: Expose modules

**Files:**

- Modify: `crates/engagement-hub/src/lib.rs`

- [x] **Step 1:** Add `pub mod audit; pub mod idempotency;`
- [x] **Step 2:** `cargo build --workspace` exits 0
- [x] **Step 3:** Commit `chore: expose audit + idempotency modules (#12)`

---

### Task 6: CI gate

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `npx markdownlint-cli2 "**/*.md" "#node_modules/**" "#target/**"`

### Deferred

- Hook `AuditWriter` and `IdempotencyChecker` into actual RPC handlers — deferred to
  T1-06 (orchestrator / StartEngagement handler implementation).
- Reconciler sweep of PENDING audit rows — deferred to T1-09.
- `StopEngagement` / `CancelEngagement` idempotency field lists — T1-10/T1-11.
- Read-RPC audit (synchronous single-phase, no PENDING) — T1-10/T1-11.
