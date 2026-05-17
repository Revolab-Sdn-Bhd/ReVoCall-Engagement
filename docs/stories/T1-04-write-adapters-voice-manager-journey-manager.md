# T1-04: Write adapters — Voice Manager + Journey Manager (with compensation semantics)

**Issue:** #11 | **Branch:** feat/11-write-adapters-voice-journey | **Date:** 2026-05-17

## Brainstorm

### Problem

T1-02 froze the port traits and request/response types. T1-03 shipped the read-side adapters and the cross-cutting policies (`with_retry`, `AdapterMetrics`, gRPC `Code` / HTTP status → typed-error mapping, panic safety via `catch_unwind`). T1-04 ships the write-side: `VoiceManagerHttpAdapter`, `VoiceManagerGrpcAdapter`, and `JourneyManagerGrpcAdapter`, plus the saga-compensation observability surface used by T1-06 and T1-07.

Five design questions surfaced before implementation. Four are settled below; one (deadline-aware retry) expands `with_retry` from T1-03 in a way the original story doc deferred.

### Q1 — Where does `request_id` for downstream idempotency live?

T1-02 froze `StartVoiceSessionReq`, `CreateExecutionReq`, `CreateTelephonyReq`, `UpdateTelephonyReq`, `IssueTestTokenReq` without a `request_id` field. PRD §12 line 1832 says writes are "idempotent via request_id" and PRD §12 line 1865 says compensation calls "carry a NEW request_id distinct from the original." Both must hold without reopening T1-02.

**Option A — Adapter generates per call (chosen).** Adapter does `Uuid::new_v4()` once before entering `with_retry`. Same id is stamped on every retry attempt's wire request (`X-Request-Id` header for HTTP; `request_id` proto field for gRPC). Within-call dedup is automatic. Compensation is a separate adapter invocation, so it gets a fresh id automatically. No change to T1-02 port types.

**Option B — Add `request_id` to port types or as a method parameter.** Reopens T1-02. Orchestrator owns the id; could correlate adapter request_ids with audit rows. More flexibility, more surface area.

**Option C — Adapter generates but echoes to caller via span context.** Same as Option A plus the id is set as an OTel span attribute (`adapter.request_id`) so traces correlate downstream logs with this attempt. We adopt this incrementally on top of Option A.

**Decision: Option A + the span-attribute hook from Option C.** Adapter mints the id; stamps it on every wire frame in `with_retry`; sets `adapter.request_id` on the active span.

**Layer separation — not to be confused with T2-10.** T2-10 (#65, `docs/rust-mirror/idempotency.md`) defines a deterministic `derive_request_id(batch_id, contact_number, attempt_number)` helper for the SDK-supplied `request_id` carried in `StartEngagementRequest`. That id is used at EH's tx-1 `ON CONFLICT (organization_id, request_id)` (PRD §7 step 3) — the **engagement's own** idempotency at the public API boundary. The adapter-minted id this story introduces is at the **downstream call** boundary (EH → VM / EH → JM). The two ids never mix; this story does not depend on T2-10.

#### Dual idempotency model — important to document

Idempotency depends on the call type. The spec must be explicit so downstream teams (VM, JM) know what guarantees to provide:

- **Create operations** — `start_voice_session`, `create_execution`, `create_telephony`, `update_telephony`, `issue_test_token`, `delete_telephony`. Downstream MUST dedup on the adapter-stamped `request_id` within a single adapter call. Both attempts of a 2-attempt `WRITE_RETRY` share the same id, so the downstream sees the second one and returns the first one's outcome.
- **Cleanup operations** — `stop_voice_session` (mode=Abort), `cancel_execution`. Downstream MUST be **state-idempotent**: "stop session V" → look up V → already stopped → return OK. The stamped `request_id` is for tracing only; it is NOT the dedup key. This is why reconciler-driven re-runs (PRD §7 lines 893–904 and §7 line 935) are safe even though they use new request_ids.

The reconciler invariant ("idempotent" at PRD §7 line 900) holds via state-idempotency for cleanup; it does not depend on adapter-level request_id stability across separate invocations.

### Q2 — VM/JM proto location

`proto/revocall/registry/v1/registry.proto` already exists in this repo (T1-03). VM and JM protos do not yet exist.

**Option A — Define locally in EH repo (chosen).** Add `proto/revocall/voice/v1/voice_manager.proto` and `proto/revocall/journey/v1/journey_manager.proto`. EH owns the consumer contract; downstream services align to it. Same pattern as T1-03 used for registry.

**Option B — Import from existing canonical source.** No canonical source today.

**Option C — Minimal stubs only.** Compromises test coverage on the gRPC wire shape.

**Decision: Option A.** The existing workspace `build.rs` at `crates/engagement-hub-adapters/build.rs` is extended to compile the two new protos (one more entry per proto in the `compile_protos` call).

### Q3 — VoiceManager HTTP endpoint contract

VM ships HTTP today (PRD §7 line 721 "today") and gRPC later. No stable HTTP API spec for VM exists in the local repos.

**Option A — EH defines the contract; downstream conforms (chosen).** T1-04 picks the endpoint paths, JSON shapes, and error envelope. The downstream VM (today: AI-Handler) is responsible for matching them. Adapter ships with `wiremock`-based tests asserting the chosen wire shape.

**Option B — Match existing AI-Handler/admin-backend endpoints.** Requires a separate research pass to extract the legacy contract; risks coupling the HTTP adapter to legacy idioms that the gRPC adapter doesn't carry.

**Option C — Skip HTTP adapter; only build gRPC.** Story AC requires both.

**Decision: Option A.** Endpoint contract:

```
POST   /v1/voice/sessions              start_voice_session
DELETE /v1/voice/sessions/{ref}        stop_voice_session?mode=abort|graceful
POST   /v1/voice/test_tokens           issue_test_token
POST   /v1/telephonies                 create_telephony
GET    /v1/telephonies?org_id=&page=   list_telephonies
GET    /v1/telephonies/{id}            get_telephony
PATCH  /v1/telephonies/{id}            update_telephony
DELETE /v1/telephonies/{id}?usage=...  delete_telephony
```

All requests carry `X-Request-Id: <uuid>`. Error envelope `{"error": {"code": "...", "message": "..."}}`. Status mapping: 4xx → `VmError::Permanent`, 503 → `VmError::Unavailable`, other 5xx → `VmError::Transient`. The chosen paths are committed in this story doc so downstream owners have a single source of truth.

### Q4 — Retry policy lives in the adapter, per method

**Option A — Baked per method in adapter (chosen).** Adapter picks the retry config statically by method: stop/cancel → `CLEANUP_RETRY` (5 attempts), creates → `WRITE_RETRY` (2 attempts), reads → `DEFAULT_RETRY` (3 attempts). Orchestrator just calls the method.

**Option B — Orchestrator passes RetryConfig in.** Reopens T1-02 traits.

**Option C — Two methods per concept (`stop_normal` vs `stop_cleanup`).** Verbose; trait surface doubles.

**Decision: Option A.** PRD §12's "*.cancel / *.stop → 5 attempts (must clean up)" is universal — compensation, AdminCancelled, and reconciler callers all benefit from the same budget for free.

**Retry policy table:**

| Trait method | Policy | Rationale |
|---|---|---|
| `VoiceManager.start_voice_session` | `WRITE_RETRY` (2) | PRD §12: writes idempotent via request_id |
| `VoiceManager.stop_voice_session` (mode=Abort) | `CLEANUP_RETRY` (5) | PRD §12: `*.stop → 5`; Abort is idempotent per the `StopMode::Abort` doc comment in `engagement-hub-ports/src/types.rs` |
| `VoiceManager.stop_voice_session` (mode=Graceful) | 1 attempt (no retry) | The `StopMode::Graceful` doc comment in `types.rs` says "Not idempotent"; PRD's "5 attempts" rule cannot apply safely. Reconciler "Overrun LIVE" sweep calls this. Branch by mode inside the adapter. |
| `VoiceManager.issue_test_token` | `DEFAULT_RETRY` (3) | Token issuance; not a saga participant |
| `VoiceManager.create_telephony` | `WRITE_RETRY` (2) | Tenant write, idempotent via request_id |
| `VoiceManager.list_telephonies` | `DEFAULT_RETRY` (3) | Read |
| `VoiceManager.get_telephony` | `DEFAULT_RETRY` (3) | Read |
| `VoiceManager.update_telephony` | `WRITE_RETRY` (2) | Tenant write |
| `VoiceManager.delete_telephony` | `WRITE_RETRY` (2) | Tenant write, NOT orchestration cleanup. PRD `*.stop / *.cancel → 5` rule does not extend to `delete_*` |
| `JourneyManager.create_execution` | `WRITE_RETRY` (2) | PRD §12: write |
| `JourneyManager.cancel_execution` | `CLEANUP_RETRY` (5) | PRD §12: `*.cancel → 5` |
| `JourneyManager.get_execution_timeline` | `DEFAULT_RETRY` (3) | Read |

### Q5 — Deadline-aware retry: in T1-04 or deferred?

T1-03 added `DeadlineContext` to `policies.rs` but `with_retry` doesn't use it. PRD §12 line 1842 requires every layer to refuse a retry if remaining < (next backoff + adapter floor). With `CLEANUP_RETRY`'s 5 attempts × up-to-2s backoff (~10s worst case), the gap is operationally significant.

**Option A — Add to `with_retry` now (chosen).** Extend signature: `with_retry(config, deadline: Option<&DeadlineContext>, target, metrics, f)`. Between attempts, short-circuit if `deadline.is_some_and(|d| d.is_too_close())` and return `E::from_deadline()`. Threads `Option<&DeadlineContext>` through every adapter method (None = unbounded, today's default — matches the adapter-default `timeout: Duration` from T1-03's Option A).

**Option B — Defer to T1-06.** Honors T1-03's original deferral. Risks the CLEANUP overshoot existing in main until T1-06 ships.

**Option C — Internal-only deadline plumbing (thread-local/span context).** Ad-hoc; harder to test.

**Decision: Option A.** Small scope creep, big correctness payoff. Requires:

1. New `FromDeadline` trait (mirrors `FromPanic`) in `engagement-hub-ports/src/error.rs` with `fn from_deadline() -> Self`.
2. New variant `DeadlineExceeded` on `RegistryError`, `JmError`, `VmError`, `PostCallError`, `AnalyticsError` (additive — same shape as the existing `InternalPanic` variant). `IsRetryable::is_retryable()` returns `false` for this variant (further retries pointless). `map_status` / `map_http_status` are untouched — `DeadlineExceeded` is emitted only by `with_retry` itself, not by status code mapping.
3. T1-03's existing read adapters get the new arg passed `None` (no behavior change).

This is the one explicitly cross-crate change in T1-04 — additive variants on the frozen error enums. Same precedent as T1-03's `InternalPanic` addition.

### Q6 — Compensation observability — where is it incremented?

PRD §7 lines 873–877 mandates `engagementhub_saga_compensation_outcome_total{stage, result}` with `stage ∈ {"jm_cancel", "vm_stop"}` and `result ∈ {"success", "transient_failure_retried", "exhausted_to_reconciler", "no_compensation_needed"}`.

**Option A — Adapter increments (rejected).** Adapter doesn't know whether a `cancel_execution(reason=AdminCancelled)` is a compensation call. For `vm_stop`, `StopMode::Abort` is overloaded — used by compensation, by AdminCancelled, and by the reconciler's "Stuck INVOKING" sweep.

**Option B — Orchestrator increments (chosen).** T1-06/T1-07 know the saga context. T1-04 ships:
1. Counter registration in `AdapterMetrics::new` with zero-init for all 2 × 4 = 8 `{stage, result}` combinations (so series exist before the first real increment — dashboards/alerts that assume series presence don't break).
2. Typed enums + a helper on `AdapterMetrics` so orchestrator code stays stringly-typed-label-free:

   ```rust
   pub enum CompensationStage   { JmCancel, VmStop }
   pub enum CompensationOutcome { Success, TransientFailureRetried, ExhaustedToReconciler, NoCompensationNeeded }
   impl AdapterMetrics {
       pub fn record_compensation(&self, stage: CompensationStage, outcome: CompensationOutcome);
   }
   ```

The asymmetry (JM could theoretically detect compensation via `CancelReason::CompensateFailedBind`) is rejected for consistency: both stages emit from the same orchestrator-side code path.

Span event `engagement-hub.saga.compensate` (PRD §7 lines 881–888) is also emitted by the orchestrator. Adapters know nothing about saga semantics.

`CompensationStage`/`CompensationOutcome` live in the **adapters crate** (`engagement_hub_adapters::saga`). T1-06 will depend on adapters, so this is consistent with existing crate boundaries.

### Q7 — Panic safety regression fix in `with_retry`

T1-03's `with_retry` writes `AssertUnwindSafe(f()).catch_unwind().await`. This catches panics during polling of the returned future, but NOT panics inside the synchronous prefix of `f()` itself (e.g., proto-request construction). Today's read adapters keep `f()` cheap (just clones), but T1-04's write adapters do construct proto request bodies inside the closure.

**Decision: fix `with_retry` so the call itself is also panic-protected.** Refactor:

```rust
let result = std::panic::AssertUnwindSafe(async {
    match f() {
        fut => fut.await,
    }
}).catch_unwind().await
   .unwrap_or_else(|_| Err(E::from_panic()));
```

(The call to `f()` is delayed until the outer future is polled; `catch_unwind` then captures any panic in either the call or the polling.)

Add a regression test that panics inside `f()`'s synchronous prefix and asserts `E::InternalPanic` is returned, not propagated.

### gRPC `Code::Cancelled` semantics — document explicitly

`registry_grpc.rs:32` maps `Code::Cancelled` → `Permanent`. T1-04 follows the same mapping for VM/JM. **Important caveat for T1-06/T1-07:** for write operations, `Cancelled` means the downstream may have processed the request or may not have — the orchestrator must treat `VmError::Permanent` / `JmError::Permanent` carrying a `Cancelled` cause as "unknown outcome → compensate," not as "downstream rejected the request." The adapter cannot distinguish these intents; T1-06's branching logic will.

### File layout decided

```
proto/revocall/voice/v1/voice_manager.proto         (NEW)
proto/revocall/journey/v1/journey_manager.proto     (NEW)
crates/engagement-hub-adapters/
  build.rs           (EXTEND: compile two more protos)
  Cargo.toml         (no new deps — reqwest, tonic, wiremock, prost already in)
  src/
    policies.rs      (EXTEND: + WRITE_RETRY, CLEANUP_RETRY consts; + deadline arg + panic-safety fix to with_retry)
    metrics.rs       (EXTEND: + saga_compensation_outcome_total counter + record_compensation helper)
    saga.rs          (NEW: CompensationStage, CompensationOutcome enums)
    voice_manager_http.rs    (NEW)
    voice_manager_grpc.rs    (NEW)
    journey_manager_grpc.rs  (NEW)
    lib.rs                   (EXTEND: pub mod the new files)
crates/engagement-hub-ports/
  src/error.rs       (EXTEND: + FromDeadline trait; + DeadlineExceeded variant on all 5 error enums)
```

### Test strategy

| Test | Tool | What it validates |
|---|---|---|
| gRPC adapter happy path | in-memory `tonic::Server` + mock | Request roundtrip, response decode |
| HTTP adapter happy path | `wiremock` | Request roundtrip, response decode |
| gRPC error code mapping | mock returns `Code::Unavailable`/`NotFound`/`InvalidArgument`/`Cancelled`/`Internal` | Each maps to the right `VmError`/`JmError` variant |
| HTTP status mapping | wiremock returns 400/404/422/503/500 | 4xx → Permanent, 503 → Unavailable, other 5xx → Transient |
| Within-call request_id reuse | Mock records seen `request_id` values across the two attempts of a transient-then-success scenario | Both attempts carry the same id |
| Cross-call request_id distinctness | One adapter instance does `start_voice_session` (records wire id A), then `stop_voice_session` (records wire id B); assert A ≠ B at the wire level | Confirms no caching/reuse across method invocations on the same struct |
| Per-method retry budget | Mock returns persistent transient; count attempts | Exactly 2 for writes, 5 for cleanup-Abort, 1 for cleanup-Graceful, 3 for reads/test_token |
| Panic safety — async body | Mock panics during polling | `with_retry` returns `E::InternalPanic` |
| Panic safety — sync prefix (NEW) | Closure panics before returning a future | `with_retry` returns `E::InternalPanic` (regression for Q7) |
| Deadline short-circuit | `with_retry` invoked with `DeadlineContext` whose remaining < 200ms; assert attempts halt and `E::DeadlineExceeded` returns | Q5 |
| Per-adapter timeout fires | Slow wiremock / slow tonic mock that holds past `adapter_default_deadline`; assert `Transient`/`DeadlineExceeded` not a hung future | Q5 plumbing |
| Saga counter registration + zero-init | Construct `AdapterMetrics::for_test()`, immediately `gather()`, assert all 8 `{stage, result}` series present with value 0 | Counter wiring |
| `record_compensation` helper | Call each enum variant; scrape; assert the right label combo incremented | Helper correctness |

Telephony CRUD: happy-path test per method (create / list / get / update / delete) + error-mapping test on one representative method.

### Out of scope (explicit deferrals)

- **gRPC SDK retries / `Retry-After` propagation** — PRD §12 talks about retry stacking across SDK + EH + downstream. T1-04 only covers EH's adapter retry; SDK-level retry is T2.
- **Audit-row correlation of adapter-level `request_id`** — adapter-minted ids are not stored in `engagement_audit`. If correlation is needed later, the orchestrator can adopt Q1's Option B incrementally.
- **`request_id` reuse for reconciler re-runs of stop/cancel** — covered by state-idempotency (Q1 dual model), not by request_id.

### External code review

This brainstorm was reviewed by an independent SWE pass (opus). Verdict: APPROVE WITH CHANGES. All 4 critical issues (StopMode::Graceful retry, dual-idempotency documentation, panic-safety sync prefix, saga counter zero-init) and 6 of 7 important issues are reflected above. The one deferred item (audit-row correlation of adapter-level request_id) is listed in "Out of scope" with a documented upgrade path.

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `JourneyManagerGrpcAdapter`, `VoiceManagerGrpcAdapter`, and `VoiceManagerHttpAdapter` with deadline-aware retries, mode-branched cleanup retry budgets, adapter-minted `request_id` idempotency, the saga compensation observability surface, and a panic-safety regression fix to `with_retry`.

**Architecture:** Three concrete adapter structs implementing the T1-02 port traits, each method going through `with_retry` (extended to take an optional `DeadlineContext`). Cross-cutting changes live in `policies.rs`, `metrics.rs`, `saga.rs` (new), and `engagement-hub-ports/src/error.rs` (additive). VM/JM gRPC clients are generated by `build.rs` from new local `.proto` files. T1-03 callsites are migrated to the new `with_retry` signature in the same PR.

**Tech Stack:** Rust 2024 edition, `tonic` for gRPC, `prost` for proto, `reqwest` for HTTP, `tokio` for async, `prometheus` for metrics, `tracing` for spans, `uuid` (v4 feature) for request_id generation, `wiremock` for HTTP test doubles, in-memory `tonic::Server` for gRPC test doubles.

### File map

```
proto/revocall/
  voice/v1/voice_manager.proto                                  CREATE
  journey/v1/journey_manager.proto                              CREATE

crates/engagement-hub-ports/src/
  error.rs                                                      MODIFY: + FromDeadline trait; + DeadlineExceeded variant on all 5 enums

crates/engagement-hub-adapters/
  build.rs                                                      MODIFY: compile two more proto paths
  src/
    lib.rs                                                      MODIFY: pub mod the new modules; pub use the new structs
    policies.rs                                                 MODIFY: + WRITE_RETRY, CLEANUP_RETRY consts; + deadline arg on with_retry; + sync-prefix panic safety
    metrics.rs                                                  MODIFY: + saga_compensation_outcome_total counter w/ zero-init; + record_compensation helper
    saga.rs                                                     CREATE: CompensationStage, CompensationOutcome enums
    journey_manager_grpc.rs                                     CREATE: JourneyManagerGrpcAdapter
    voice_manager_grpc.rs                                       CREATE: VoiceManagerGrpcAdapter
    voice_manager_http.rs                                       CREATE: VoiceManagerHttpAdapter
    registry_grpc.rs                                            MODIFY: migrate with_retry callsites (insert `None,`)
    post_call_http.rs                                           MODIFY: migrate with_retry callsites
    analytics_http.rs                                           MODIFY: migrate with_retry callsites
    registry_stub.rs                                            MODIFY: migrate with_retry callsites
```

### Working directory

All commands run from `/Users/chunzhe/Projects/ReVoCall-Engagement.t1-04` (the worktree). Branch: `feat/11-write-adapters-voice-journey`.

---

### Task 1: Add `FromDeadline` trait + `DeadlineExceeded` variant to all error enums

**Files:**
- Modify: `crates/engagement-hub-ports/src/error.rs`

- [ ] **Step 1: Add the `FromDeadline` trait next to `FromPanic`**

In `engagement-hub-ports/src/error.rs`, add after the `FromPanic` trait:

```rust
/// Implemented by error types that can represent a deadline-exceeded outcome
/// from `with_retry` short-circuiting before the next attempt.
pub trait FromDeadline {
    fn from_deadline() -> Self;
}
```

- [ ] **Step 2: Add `DeadlineExceeded` variant + trait impls to each of the 5 error enums**

For each of `RegistryError`, `JmError`, `VmError`, `PostCallError`, `AnalyticsError`:

1. Add the variant:

```rust
    #[error("deadline exceeded — refused retry")]
    DeadlineExceeded,
```

2. Add the `FromDeadline` impl:

```rust
impl FromDeadline for RegistryError {
    fn from_deadline() -> Self {
        Self::DeadlineExceeded
    }
}
```

(Repeat for `JmError`, `VmError`, `PostCallError`, `AnalyticsError` — replace the type name in each impl.)

3. The existing `IsRetryable::is_retryable` impls must NOT match `DeadlineExceeded` as retryable. Their current shape is:

```rust
matches!(self, Self::Transient(_) | Self::Unavailable)
```

This already excludes `DeadlineExceeded` by being non-exhaustive in the positive sense — but verify the `matches!` expression for each error type and confirm it does NOT include the new variant.

- [ ] **Step 3: Build the ports crate to confirm exhaustive-match warnings are clean**

```bash
cargo build -p engagement-hub-ports
```

Expected: success, zero warnings.

- [ ] **Step 4: Run ports crate tests to ensure nothing broke**

```bash
cargo test -p engagement-hub-ports
```

Expected: all existing tests pass (107+ unit tests from T1-02 fakes).

- [ ] **Step 5: Commit**

```bash
git add crates/engagement-hub-ports/src/error.rs
git commit -m "ports: add FromDeadline trait + DeadlineExceeded variant on all error enums

Additive change matching the FromPanic / InternalPanic pattern. is_retryable
returns false for DeadlineExceeded — once with_retry refuses the next
attempt, further retries are pointless.

Refs #11"
```

---

### Task 2: Extend `with_retry` — deadline + sync-prefix panic safety + new RetryConfig consts

**Files:**
- Modify: `crates/engagement-hub-adapters/src/policies.rs`

- [ ] **Step 1: Write the failing test for `WRITE_RETRY` and `CLEANUP_RETRY` consts**

Append to the `tests` module in `policies.rs`:

```rust
#[test]
fn write_retry_is_two_attempts() {
    assert_eq!(WRITE_RETRY.max_attempts, 2);
}

#[test]
fn cleanup_retry_is_five_attempts() {
    assert_eq!(CLEANUP_RETRY.max_attempts, 5);
}
```

- [ ] **Step 2: Run, expect fail (consts not defined)**

```bash
cargo test -p engagement-hub-adapters policies::tests::write_retry_is_two_attempts -- --nocapture
```

Expected: compile error / undefined `WRITE_RETRY`.

- [ ] **Step 3: Add the consts in `policies.rs`**

Insert after the existing `DEFAULT_RETRY` const:

```rust
/// 2 attempts — used for write operations whose downstream idempotency comes
/// from a per-call `request_id`. PRD §12: writes idempotent via request_id.
pub const WRITE_RETRY: RetryConfig = RetryConfig {
    max_attempts: 2,
    initial_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_secs(2),
};

/// 5 attempts — used for cleanup operations (`*.stop`/`*.cancel`) that MUST
/// clean up downstream resources. PRD §12 saga compensation budget.
pub const CLEANUP_RETRY: RetryConfig = RetryConfig {
    max_attempts: 5,
    initial_backoff: Duration::from_millis(50),
    max_backoff: Duration::from_secs(2),
};
```

- [ ] **Step 4: Run, expect pass**

```bash
cargo test -p engagement-hub-adapters policies::tests::write_retry_is_two_attempts policies::tests::cleanup_retry_is_five_attempts
```

Expected: 2 passed.

- [ ] **Step 5: Write the failing test for sync-prefix panic safety**

Append to the `tests` module:

```rust
#[tokio::test]
async fn catches_panic_in_synchronous_closure_prefix() {
    // The closure panics BEFORE returning a future. Today's with_retry
    // (pre-T1-04) only wraps the returned future in catch_unwind, so this
    // panic would escape. The fix wraps the call to f() itself.
    let r: Result<i32, E> = with_retry(no_sleep_config(1), None, "t", None, || {
        panic!("sync-prefix panic");
        #[allow(unreachable_code)]
        async move { Ok::<i32, E>(0) }
    })
    .await;
    assert_eq!(r, Err(E::Panic));
}
```

Note the new `None` argument for the deadline param — this test also enforces the new signature.

- [ ] **Step 6: Write the failing test for deadline short-circuit**

Append:

```rust
#[tokio::test]
async fn deadline_too_close_short_circuits_before_next_attempt() {
    let ctx = DeadlineContext::from_remaining(
        Duration::from_millis(100),
        Duration::from_secs(5),
    );
    // is_too_close()==true; first attempt should run, but no retry should be attempted.
    let n = Arc::new(AtomicU32::new(0));
    let c = n.clone();
    let r: Result<i32, E> = with_retry(
        no_sleep_config(3),
        Some(&ctx),
        "t",
        None,
        || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(E::Transient)
            }
        },
    )
    .await;
    // First attempt ran, deadline check fires before attempt 2.
    assert_eq!(n.load(Ordering::SeqCst), 1);
    assert_eq!(r, Err(E::Deadline));
}
```

Also extend the test-local error type with `Deadline` variant and `FromDeadline` impl at the top of the `tests` module:

```rust
#[derive(Debug, PartialEq, Clone)]
enum E {
    Transient,
    Permanent,
    Panic,
    Deadline,
}
impl IsRetryable for E {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient)
    }
}
impl FromPanic for E {
    fn from_panic() -> Self { Self::Panic }
}
impl FromDeadline for E {
    fn from_deadline() -> Self { Self::Deadline }
}
```

(Replace the existing test-local `E` and its impls.)

- [ ] **Step 7: Update all existing tests in `policies.rs` to pass `None` for the new deadline arg**

Find every `with_retry(no_sleep_config(...)` call in the tests module and insert `None,` between the config and the target string. Example:

```rust
// Before
let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || { ... }).await;

// After
let r: Result<i32, E> = with_retry(no_sleep_config(3), None, "t", None, || { ... }).await;
```

- [ ] **Step 8: Run the test suite, expect 2 new failures (sync-prefix panic + deadline) and all other tests to compile and pass**

```bash
cargo test -p engagement-hub-adapters policies
```

Expected: catches_panic_in_synchronous_closure_prefix FAIL, deadline_too_close_short_circuits_before_next_attempt FAIL, all others PASS.

- [ ] **Step 9: Modify `with_retry` to accept `deadline: Option<&DeadlineContext>` and use the two-step panic catch**

In `policies.rs`, replace the existing `with_retry` body:

```rust
pub async fn with_retry<F, Fut, T, E>(
    config: RetryConfig,
    deadline: Option<&DeadlineContext>,
    target: &str,
    metrics: Option<&AdapterMetrics>,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
    E: IsRetryable + FromPanic + FromDeadline + Send + 'static,
    T: Send + 'static,
{
    debug_assert!(
        config.max_attempts > 0,
        "RetryConfig::max_attempts must be > 0"
    );
    let mut backoff = config.initial_backoff;
    for attempt in 0..config.max_attempts {
        // Two-stage panic catch: first wrap the synchronous call to f() so a
        // panic in the closure's sync prefix (e.g. proto request construction)
        // is converted to E::from_panic(); then wrap the returned future so
        // panics during polling are also caught.
        let result = match std::panic::catch_unwind(AssertUnwindSafe(&mut f)) {
            Ok(fut) => AssertUnwindSafe(fut)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(E::from_panic())),
            Err(_) => Err(E::from_panic()),
        };

        match &result {
            Err(e) if e.is_retryable() && attempt + 1 < config.max_attempts => {
                if let Some(m) = metrics {
                    m.retries_total
                        .with_label_values(&[target, &(attempt + 1).to_string()])
                        .inc();
                }
                // Deadline gate before next attempt.
                if let Some(d) = deadline {
                    if d.is_too_close() {
                        if let Some(m) = metrics {
                            m.deadline_exceeded_total
                                .with_label_values(&[target])
                                .inc();
                        }
                        return Err(E::from_deadline());
                    }
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
```

Note: `catch_unwind(AssertUnwindSafe(&mut f))` calls `(&mut f)()` (the `FnMut` impl) inside a sync `catch_unwind`. This catches panics in the synchronous prefix of the closure body.

Also add the new trait import at the top of the file:

```rust
use engagement_hub_ports::error::{FromDeadline, FromPanic, IsRetryable};
```

- [ ] **Step 10: Run the test suite, expect all to pass**

```bash
cargo test -p engagement-hub-adapters policies
```

Expected: all 12+ tests PASS.

- [ ] **Step 11: Commit**

```bash
git add crates/engagement-hub-adapters/src/policies.rs
git commit -m "policies: add deadline-aware retry, sync-prefix panic safety, WRITE/CLEANUP_RETRY consts

with_retry now accepts Option<&DeadlineContext> and short-circuits with
E::from_deadline() between attempts when remaining < 200ms floor. The
synchronous prefix of the closure (proto request construction, etc.) is
also panic-safe via std::panic::catch_unwind around the call to f().

Refs #11"
```

---

### Task 3: Migrate T1-03 callsites to the new `with_retry` signature

**Files:**
- Modify: `crates/engagement-hub-adapters/src/registry_grpc.rs`
- Modify: `crates/engagement-hub-adapters/src/post_call_http.rs`
- Modify: `crates/engagement-hub-adapters/src/analytics_http.rs`
- Modify: `crates/engagement-hub-adapters/src/registry_stub.rs` (if it uses with_retry)

- [ ] **Step 1: Find all callsites**

```bash
grep -n "with_retry(" /Users/chunzhe/Projects/ReVoCall-Engagement.t1-04/crates/engagement-hub-adapters/src/*.rs | grep -v policies.rs
```

Expected: ~10–15 callsites across the four files.

- [ ] **Step 2: At each callsite, insert `None,` between the config and the target string**

Example (registry_grpc.rs):

```rust
// Before
with_retry(
    REGISTRY_RESOLVE_RETRY,
    "registry",
    Some(&metrics),
    move || { ... },
)

// After
with_retry(
    REGISTRY_RESOLVE_RETRY,
    None,                 // deadline — wired in T1-06
    "registry",
    Some(&metrics),
    move || { ... },
)
```

Apply this to every callsite. None of the T1-03 adapters currently know about deadlines.

- [ ] **Step 3: Build to verify**

```bash
cargo build -p engagement-hub-adapters
```

Expected: compiles with zero warnings.

- [ ] **Step 4: Run the full adapters test suite**

```bash
cargo test -p engagement-hub-adapters
```

Expected: all existing T1-03 tests PASS unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/engagement-hub-adapters/src/registry_grpc.rs \
        crates/engagement-hub-adapters/src/post_call_http.rs \
        crates/engagement-hub-adapters/src/analytics_http.rs \
        crates/engagement-hub-adapters/src/registry_stub.rs
git commit -m "adapters: migrate T1-03 callsites to with_retry's new deadline arg

Passes None for the deadline parameter; behavior unchanged. T1-06 will
wire actual deadlines through DeadlineContext::from_remaining when the
orchestrator threads request deadlines down.

Refs #11"
```

---

### Task 4: Add `saga.rs` module with `CompensationStage` / `CompensationOutcome`

**Files:**
- Create: `crates/engagement-hub-adapters/src/saga.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/engagement-hub-adapters/src/saga.rs`:

```rust
//! Saga compensation observability primitives.
//!
//! These enums identify the stage (which downstream — VM or JM) and the
//! outcome of a compensation attempt for the `engagementhub_saga_compensation_outcome_total`
//! Prometheus counter (PRD §7).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationStage {
    /// Compensating after a failed StartEngagement bind by cancelling the
    /// journey execution that did succeed.
    JmCancel,
    /// Compensating after a failed StartEngagement bind by stopping the
    /// voice session that did succeed.
    VmStop,
}

impl CompensationStage {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::JmCancel => "jm_cancel",
            Self::VmStop => "vm_stop",
        }
    }

    pub const ALL: [Self; 2] = [Self::JmCancel, Self::VmStop];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationOutcome {
    Success,
    TransientFailureRetried,
    ExhaustedToReconciler,
    NoCompensationNeeded,
}

impl CompensationOutcome {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::TransientFailureRetried => "transient_failure_retried",
            Self::ExhaustedToReconciler => "exhausted_to_reconciler",
            Self::NoCompensationNeeded => "no_compensation_needed",
        }
    }

    pub const ALL: [Self; 4] = [
        Self::Success,
        Self::TransientFailureRetried,
        Self::ExhaustedToReconciler,
        Self::NoCompensationNeeded,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_labels_match_prd_spec() {
        assert_eq!(CompensationStage::JmCancel.as_label(), "jm_cancel");
        assert_eq!(CompensationStage::VmStop.as_label(), "vm_stop");
    }

    #[test]
    fn outcome_labels_match_prd_spec() {
        assert_eq!(CompensationOutcome::Success.as_label(), "success");
        assert_eq!(
            CompensationOutcome::TransientFailureRetried.as_label(),
            "transient_failure_retried"
        );
        assert_eq!(
            CompensationOutcome::ExhaustedToReconciler.as_label(),
            "exhausted_to_reconciler"
        );
        assert_eq!(
            CompensationOutcome::NoCompensationNeeded.as_label(),
            "no_compensation_needed"
        );
    }

    #[test]
    fn all_combinations_covered() {
        assert_eq!(CompensationStage::ALL.len(), 2);
        assert_eq!(CompensationOutcome::ALL.len(), 4);
    }
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Append to `crates/engagement-hub-adapters/src/lib.rs`:

```rust
pub mod saga;
```

- [ ] **Step 3: Run the new tests**

```bash
cargo test -p engagement-hub-adapters saga::
```

Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/src/saga.rs crates/engagement-hub-adapters/src/lib.rs
git commit -m "saga: add CompensationStage and CompensationOutcome enums

Typed labels for the engagementhub_saga_compensation_outcome_total counter.
Used by the orchestrator (T1-06/T1-07) when emitting saga compensation
metrics. Label strings match PRD §7 verbatim.

Refs #11"
```

---

### Task 5: Extend `AdapterMetrics` with the saga counter (zero-init) + `record_compensation` helper

**Files:**
- Modify: `crates/engagement-hub-adapters/src/metrics.rs`

- [ ] **Step 1: Write failing tests**

Replace/extend the `tests` module in `metrics.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::saga::{CompensationOutcome, CompensationStage};
    use prometheus::Encoder;

    fn gather_text(r: &Registry) -> String {
        let enc = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        enc.encode(&r.gather(), &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn registers_all_counters() {
        let r = Registry::new();
        let m = AdapterMetrics::new(&r).unwrap();
        m.retries_total.with_label_values(&["registry", "1"]).inc();
        m.deadline_exceeded_total
            .with_label_values(&["registry"])
            .inc();
        let text = gather_text(&r);
        assert!(text.contains("engagementhub_adapter_retries_total"));
        assert!(text.contains("engagementhub_deadline_exceeded_total"));
        assert!(text.contains("engagementhub_saga_compensation_outcome_total"));
    }

    #[test]
    fn saga_counter_is_zero_initialized_for_all_label_combinations() {
        let r = Registry::new();
        let _m = AdapterMetrics::new(&r).unwrap();
        let text = gather_text(&r);
        // All 2 × 4 = 8 series must be present with value 0.
        for stage in CompensationStage::ALL {
            for outcome in CompensationOutcome::ALL {
                let expected = format!(
                    "engagementhub_saga_compensation_outcome_total{{result=\"{}\",stage=\"{}\"}} 0",
                    outcome.as_label(),
                    stage.as_label()
                );
                assert!(
                    text.contains(&expected),
                    "missing zero-init for stage={:?} outcome={:?}\n--- gather text ---\n{}",
                    stage,
                    outcome,
                    text,
                );
            }
        }
    }

    #[test]
    fn record_compensation_increments_correct_series() {
        let r = Registry::new();
        let m = AdapterMetrics::new(&r).unwrap();
        m.record_compensation(CompensationStage::JmCancel, CompensationOutcome::Success);
        m.record_compensation(CompensationStage::VmStop, CompensationOutcome::ExhaustedToReconciler);
        let text = gather_text(&r);
        assert!(text.contains(
            "engagementhub_saga_compensation_outcome_total{result=\"success\",stage=\"jm_cancel\"} 1"
        ));
        assert!(text.contains(
            "engagementhub_saga_compensation_outcome_total{result=\"exhausted_to_reconciler\",stage=\"vm_stop\"} 1"
        ));
    }
}
```

- [ ] **Step 2: Run, expect compile fail (no `saga_compensation_outcome_total` field, no `record_compensation`)**

```bash
cargo test -p engagement-hub-adapters metrics::tests
```

Expected: compile error.

- [ ] **Step 3: Implement the counter + helper**

Replace the `AdapterMetrics` struct and `new` constructor in `metrics.rs`:

```rust
use std::sync::Arc;

use anyhow::Result;
use prometheus::{IntCounterVec, Opts, Registry};

use crate::saga::{CompensationOutcome, CompensationStage};

pub struct AdapterMetrics {
    pub retries_total: IntCounterVec,
    pub deadline_exceeded_total: IntCounterVec,
    pub saga_compensation_outcome_total: IntCounterVec,
}

impl AdapterMetrics {
    pub fn new(registry: &Registry) -> Result<Arc<Self>> {
        let retries_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_adapter_retries_total",
                "Retry attempts per adapter target and attempt number",
            ),
            &["target", "attempt"],
        )?;
        registry.register(Box::new(retries_total.clone()))?;

        let deadline_exceeded_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_deadline_exceeded_total",
                "Adapter calls refused due to deadline too close",
            ),
            &["target"],
        )?;
        registry.register(Box::new(deadline_exceeded_total.clone()))?;

        let saga_compensation_outcome_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_saga_compensation_outcome_total",
                "Saga compensation attempt outcomes by stage (PRD §7)",
            ),
            &["stage", "result"],
        )?;
        registry.register(Box::new(saga_compensation_outcome_total.clone()))?;

        // Zero-init all 2 × 4 series so dashboards/alerts that assume series
        // presence don't break before the first real increment.
        for stage in CompensationStage::ALL {
            for outcome in CompensationOutcome::ALL {
                saga_compensation_outcome_total
                    .with_label_values(&[stage.as_label(), outcome.as_label()])
                    .inc_by(0);
            }
        }

        Ok(Arc::new(Self {
            retries_total,
            deadline_exceeded_total,
            saga_compensation_outcome_total,
        }))
    }

    pub fn record_compensation(&self, stage: CompensationStage, outcome: CompensationOutcome) {
        self.saga_compensation_outcome_total
            .with_label_values(&[stage.as_label(), outcome.as_label()])
            .inc();
    }

    /// Returns a metrics instance backed by a throwaway registry (for tests).
    pub fn for_test() -> Arc<Self> {
        Self::new(&Registry::new()).expect("test metrics")
    }
}
```

- [ ] **Step 4: Run all metrics tests, expect pass**

```bash
cargo test -p engagement-hub-adapters metrics::tests
```

Expected: 3 PASS.

- [ ] **Step 5: Run full adapters suite to confirm no regression**

```bash
cargo test -p engagement-hub-adapters
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/engagement-hub-adapters/src/metrics.rs
git commit -m "metrics: add saga_compensation_outcome_total counter + record_compensation helper

Zero-initializes all 2 × 4 {stage, result} label combinations at
AdapterMetrics::new so dashboards/alerts that assume series presence
don't break before the first real increment from T1-06/T1-07.

Refs #11"
```

---

### Task 6: Add `journey_manager.proto`

**Files:**
- Create: `proto/revocall/journey/v1/journey_manager.proto`

- [ ] **Step 1: Write the proto file**

Create `proto/revocall/journey/v1/journey_manager.proto`:

```protobuf
syntax = "proto3";

package revocall.journey.v1;

// JourneyManager is the gRPC contract Engagement Hub's
// JourneyManagerGrpcAdapter calls into. Defined here so EH owns the
// consumer contract; downstream services align to it.

service JourneyManager {
  rpc CreateExecution(CreateExecutionRequest) returns (CreateExecutionResponse);
  rpc CancelExecution(CancelExecutionRequest) returns (CancelExecutionResponse);
  rpc GetExecutionTimeline(GetExecutionTimelineRequest) returns (GetExecutionTimelineResponse);
}

message CreateExecutionRequest {
  // Adapter-minted UUIDv4. Same id is stamped on every retry attempt
  // within a single adapter call (downstream MUST dedup on this).
  string request_id = 1;
  string journey_version = 2;
  string org_id = 3;
  string engagement_id = 4;
}

message CreateExecutionResponse {
  ExecutionRefProto execution_ref = 1;
}

message ExecutionRefProto {
  string id = 1;  // UUID string
}

message CancelExecutionRequest {
  string request_id = 1;
  ExecutionRefProto execution_ref = 2;
  CancelReason reason = 3;
}

enum CancelReason {
  CANCEL_REASON_UNSPECIFIED = 0;
  CANCEL_REASON_COMPENSATE_FAILED_BIND = 1;
  CANCEL_REASON_USER_REQUESTED = 2;
  CANCEL_REASON_ORCHESTRATOR_TIMEOUT = 3;
  CANCEL_REASON_ADMIN_CANCELLED = 4;
}

message CancelExecutionResponse {}

message GetExecutionTimelineRequest {
  string request_id = 1;
  ExecutionRefProto execution_ref = 2;
  optional uint64 after_sequence = 3;
}

message GetExecutionTimelineResponse {
  repeated TimelineEventProto events = 1;
}

message TimelineEventProto {
  uint64 sequence = 1;
  string kind = 2;
}
```

- [ ] **Step 2: Commit (build wiring comes in Task 8)**

```bash
git add proto/revocall/journey/v1/journey_manager.proto
git commit -m "proto: add journey_manager.proto for JourneyManagerGrpcAdapter

Defines the gRPC contract EH consumes. CancelReason mirrors the
engagement_hub_ports::types::CancelReason variants so the adapter can
map 1:1.

Refs #11"
```

---

### Task 7: Add `voice_manager.proto`

**Files:**
- Create: `proto/revocall/voice/v1/voice_manager.proto`

- [ ] **Step 1: Write the proto file**

Create `proto/revocall/voice/v1/voice_manager.proto`:

```protobuf
syntax = "proto3";

package revocall.voice.v1;

service VoiceManager {
  rpc StartVoiceSession(StartVoiceSessionRequest) returns (StartVoiceSessionResponse);
  rpc StopVoiceSession(StopVoiceSessionRequest) returns (StopVoiceSessionResponse);
  rpc IssueTestToken(IssueTestTokenRequest) returns (IssueTestTokenResponse);

  rpc CreateTelephony(CreateTelephonyRequest) returns (CreateTelephonyResponse);
  rpc ListTelephonies(ListTelephoniesRequest) returns (ListTelephoniesResponse);
  rpc GetTelephony(GetTelephonyRequest) returns (GetTelephonyResponse);
  rpc UpdateTelephony(UpdateTelephonyRequest) returns (UpdateTelephonyResponse);
  rpc DeleteTelephony(DeleteTelephonyRequest) returns (DeleteTelephonyResponse);
}

// ---- Voice session ----

message StartVoiceSessionRequest {
  string request_id = 1;
  string engagement_id = 2;
  string org_id = 3;
}

message StartVoiceSessionResponse {
  VoiceSessionRefProto session_ref = 1;
}

message VoiceSessionRefProto {
  string id = 1;  // UUID string
}

message StopVoiceSessionRequest {
  string request_id = 1;
  VoiceSessionRefProto session_ref = 2;
  StopMode mode = 3;
}

enum StopMode {
  STOP_MODE_UNSPECIFIED = 0;
  STOP_MODE_ABORT = 1;
  STOP_MODE_GRACEFUL = 2;
}

message StopVoiceSessionResponse {}

// ---- Test token ----

message IssueTestTokenRequest {
  string request_id = 1;
  string org_id = 2;
}

message IssueTestTokenResponse {
  string token = 1;
}

// ---- Telephony CRUD ----

message TelephonyProto {
  string id = 1;       // UUID string
  string org_id = 2;
  string phone_number = 3;
}

message CreateTelephonyRequest {
  string request_id = 1;
  string org_id = 2;
  string phone_number = 3;
}

message CreateTelephonyResponse {
  TelephonyProto telephony = 1;
}

message ListTelephoniesRequest {
  string request_id = 1;
  string org_id = 2;
  optional string page_token = 3;
}

message ListTelephoniesResponse {
  repeated TelephonyProto telephonies = 1;
}

message GetTelephonyRequest {
  string request_id = 1;
  string telephony_id = 2;
}

message GetTelephonyResponse {
  TelephonyProto telephony = 1;
}

message UpdateTelephonyRequest {
  string request_id = 1;
  string telephony_id = 2;
  string phone_number = 3;
}

message UpdateTelephonyResponse {
  TelephonyProto telephony = 1;
}

message DeleteTelephonyRequest {
  string request_id = 1;
  string telephony_id = 2;
  string usage = 3;
}

message DeleteTelephonyResponse {}
```

- [ ] **Step 2: Commit**

```bash
git add proto/revocall/voice/v1/voice_manager.proto
git commit -m "proto: add voice_manager.proto for VoiceManagerGrpcAdapter

Defines the gRPC contract EH consumes for VM. Same wire shape as the
HTTP adapter — request_id field corresponds to X-Request-Id header.

Refs #11"
```

---

### Task 8: Extend `build.rs` to compile the two new protos

**Files:**
- Modify: `crates/engagement-hub-adapters/build.rs`

- [ ] **Step 1: Update `build.rs`**

Replace the contents of `crates/engagement-hub-adapters/build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // build.rs runs with the crate dir as CWD; navigate up to the workspace root
    // so that proto paths resolve correctly.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .expect("could not locate workspace root")
        .to_path_buf();

    let include_path = workspace_root.join("proto");
    let protos = [
        workspace_root.join("proto/revocall/registry/v1/registry.proto"),
        workspace_root.join("proto/revocall/journey/v1/journey_manager.proto"),
        workspace_root.join("proto/revocall/voice/v1/voice_manager.proto"),
    ];

    tonic_build::configure()
        .build_server(true)
        .compile_protos(&protos, &[include_path])?;
    Ok(())
}
```

- [ ] **Step 2: Build the adapters crate to confirm proto codegen works**

```bash
cargo build -p engagement-hub-adapters
```

Expected: success. The generated modules `revocall.journey.v1` and `revocall.voice.v1` will appear in `target/.../OUT_DIR/`.

- [ ] **Step 3: Run all adapter tests to confirm no regression**

```bash
cargo test -p engagement-hub-adapters
```

Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/build.rs
git commit -m "build: compile voice/v1 and journey/v1 proto modules

Refs #11"
```

---

### Task 9: `JourneyManagerGrpcAdapter` skeleton + `map_status`

**Files:**
- Create: `crates/engagement-hub-adapters/src/journey_manager_grpc.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

- [ ] **Step 1: Create the file with skeleton + `map_status`**

Create `crates/engagement-hub-adapters/src/journey_manager_grpc.rs`:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::JmError,
    ports::JourneyManagerPort,
    types::{
        CancelReason, CreateExecutionReq, ExecutionRef, Timeline, TimelineEvent, TimelineOpts,
    },
};
use tonic::{Code, transport::Channel};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{CLEANUP_RETRY, DEFAULT_RETRY, WRITE_RETRY, with_retry},
};

mod proto {
    tonic::include_proto!("revocall.journey.v1");
}
use proto::journey_manager_client::JourneyManagerClient;

fn map_status(s: tonic::Status) -> JmError {
    match s.code() {
        Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::Cancelled => JmError::Permanent(format!("{:?}: {}", s.code(), s.message())),
        Code::Unavailable => JmError::Unavailable,
        _ => JmError::Transient(format!("{:?}: {}", s.code(), s.message())),
    }
}

fn cancel_reason_to_proto(r: CancelReason) -> proto::CancelReason {
    match r {
        CancelReason::CompensateFailedBind => proto::CancelReason::CompensateFailedBind,
        CancelReason::UserRequested => proto::CancelReason::UserRequested,
        CancelReason::OrchestratorTimeout => proto::CancelReason::OrchestratorTimeout,
        CancelReason::AdminCancelled => proto::CancelReason::AdminCancelled,
    }
}

pub struct JourneyManagerGrpcAdapter {
    client: JourneyManagerClient<Channel>,
    metrics: Arc<AdapterMetrics>,
}

impl JourneyManagerGrpcAdapter {
    pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client: JourneyManagerClient::new(channel),
            metrics,
        }
    }
}

// Trait impl methods filled in in Tasks 10–12.

#[cfg(test)]
mod tests {
    // Shared test harness — populated as methods are implemented.
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Append to `crates/engagement-hub-adapters/src/lib.rs`:

```rust
pub mod journey_manager_grpc;
pub use journey_manager_grpc::JourneyManagerGrpcAdapter;
```

- [ ] **Step 3: Build**

```bash
cargo build -p engagement-hub-adapters
```

Expected: success (compiles, no trait impl yet so `JourneyManagerGrpcAdapter` does not implement `JourneyManagerPort` — that's fine for now).

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/src/journey_manager_grpc.rs \
        crates/engagement-hub-adapters/src/lib.rs
git commit -m "journey: scaffold JourneyManagerGrpcAdapter (struct + map_status)

Trait impl methods land in subsequent commits.

Refs #11"
```

---

### Task 10: `JourneyManagerGrpcAdapter::create_execution` (TDD)

**Files:**
- Modify: `crates/engagement-hub-adapters/src/journey_manager_grpc.rs`

- [ ] **Step 1: Write the mock + test harness inside the file's `tests` module**

Append to the `tests` module:

```rust
use super::*;
use engagement_hub_ports::types::EngagementId;
use proto::{
    journey_manager_server::{JourneyManager as JmServer, JourneyManagerServer},
    CancelExecutionRequest, CancelExecutionResponse, CreateExecutionRequest,
    CreateExecutionResponse, ExecutionRefProto, GetExecutionTimelineRequest,
    GetExecutionTimelineResponse,
};
use std::sync::Mutex;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status, transport::Server};

#[derive(Default)]
struct CallCounters {
    create: Mutex<u32>,
    cancel: Mutex<u32>,
    timeline: Mutex<u32>,
}

struct MockJm {
    create_result: Mutex<Result<ExecutionRefProto, Status>>,
    cancel_result: Mutex<Result<(), Status>>,
    timeline_result: Mutex<Result<Vec<proto::TimelineEventProto>, Status>>,
    seen_request_ids: Mutex<Vec<String>>,
    counters: CallCounters,
}

impl MockJm {
    fn always_ok_create(ref_id: Uuid) -> Self {
        Self {
            create_result: Mutex::new(Ok(ExecutionRefProto {
                id: ref_id.to_string(),
            })),
            cancel_result: Mutex::new(Ok(())),
            timeline_result: Mutex::new(Ok(vec![])),
            seen_request_ids: Mutex::new(vec![]),
            counters: CallCounters::default(),
        }
    }
}

#[tonic::async_trait]
impl JmServer for MockJm {
    async fn create_execution(
        &self,
        req: Request<CreateExecutionRequest>,
    ) -> Result<Response<CreateExecutionResponse>, Status> {
        *self.counters.create.lock().unwrap() += 1;
        self.seen_request_ids
            .lock()
            .unwrap()
            .push(req.into_inner().request_id);
        let r = self
            .create_result
            .lock()
            .unwrap()
            .as_ref()
            .map(|x| x.clone())
            .map_err(|e| e.clone())?;
        Ok(Response::new(CreateExecutionResponse {
            execution_ref: Some(r),
        }))
    }

    async fn cancel_execution(
        &self,
        req: Request<CancelExecutionRequest>,
    ) -> Result<Response<CancelExecutionResponse>, Status> {
        *self.counters.cancel.lock().unwrap() += 1;
        self.seen_request_ids
            .lock()
            .unwrap()
            .push(req.into_inner().request_id);
        self
            .cancel_result
            .lock()
            .unwrap()
            .as_ref()
            .map(|_| ())
            .map_err(|e| e.clone())?;
        Ok(Response::new(CancelExecutionResponse {}))
    }

    async fn get_execution_timeline(
        &self,
        req: Request<GetExecutionTimelineRequest>,
    ) -> Result<Response<GetExecutionTimelineResponse>, Status> {
        *self.counters.timeline.lock().unwrap() += 1;
        self.seen_request_ids
            .lock()
            .unwrap()
            .push(req.into_inner().request_id);
        let events = self
            .timeline_result
            .lock()
            .unwrap()
            .as_ref()
            .map(|v| v.clone())
            .map_err(|e| e.clone())?;
        Ok(Response::new(GetExecutionTimelineResponse { events }))
    }
}

async fn start_server(mock: Arc<MockJm>) -> Channel {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(JourneyManagerServer::from_arc(mock))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap()
}

#[tokio::test]
async fn create_execution_happy_path() {
    let exec_id = Uuid::new_v4();
    let mock = Arc::new(MockJm::always_ok_create(exec_id));
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let r = adapter
        .create_execution(CreateExecutionReq {
            journey_version: "v1".into(),
            org_id: "org-1".into(),
            engagement_id: EngagementId::default(),
        })
        .await
        .expect("ok");
    assert_eq!(r.as_uuid(), &exec_id);
    // request_id was stamped (non-empty UUID string).
    let ids = mock.seen_request_ids.lock().unwrap();
    assert_eq!(ids.len(), 1);
    Uuid::parse_str(&ids[0]).expect("stamped request_id is a UUID");
}
```

- [ ] **Step 2: Run, expect compile fail (`create_execution` not yet a method on `JourneyManagerPort` for the adapter)**

```bash
cargo test -p engagement-hub-adapters journey_manager_grpc::tests::create_execution_happy_path
```

Expected: `JourneyManagerGrpcAdapter does not implement JourneyManagerPort` (or `create_execution not found`).

- [ ] **Step 3: Implement `create_execution`**

Above the `#[cfg(test)] mod tests`, add the trait impl block:

```rust
#[async_trait]
impl JourneyManagerPort for JourneyManagerGrpcAdapter {
    async fn create_execution(
        &self,
        req: CreateExecutionReq,
    ) -> Result<ExecutionRef, JmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "journey_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::CreateExecutionRequest {
                request_id: request_id.clone(),
                journey_version: req.journey_version.clone(),
                org_id: req.org_id.clone(),
                engagement_id: req.engagement_id.to_string(),
            };
            async move {
                c.create_execution(r)
                    .await
                    .map_err(map_status)
                    .and_then(|resp| {
                        let er = resp
                            .into_inner()
                            .execution_ref
                            .ok_or_else(|| {
                                JmError::Permanent("journey_manager: empty execution_ref".into())
                            })?;
                        let uid = er.id.parse::<Uuid>().map_err(|e| {
                            JmError::Permanent(format!("bad execution_ref uuid: {e}"))
                        })?;
                        Ok(ExecutionRef::new(uid))
                    })
            }
        })
        .await
    }

    async fn cancel_execution(
        &self,
        _ref_: &ExecutionRef,
        _reason: CancelReason,
    ) -> Result<(), JmError> {
        // Implemented in Task 11.
        unimplemented!("see Task 11")
    }

    async fn get_execution_timeline(
        &self,
        _ref_: &ExecutionRef,
        _opts: TimelineOpts,
    ) -> Result<Timeline, JmError> {
        // Implemented in Task 12.
        unimplemented!("see Task 12")
    }
}
```

- [ ] **Step 4: Run the test, expect pass**

```bash
cargo test -p engagement-hub-adapters journey_manager_grpc::tests::create_execution_happy_path
```

Expected: PASS.

- [ ] **Step 5: Add error-mapping test**

Append to the `tests` module:

```rust
#[tokio::test]
async fn create_execution_invalid_argument_maps_to_permanent() {
    let mock = Arc::new(MockJm {
        create_result: Mutex::new(Err(Status::invalid_argument("bad journey_version"))),
        cancel_result: Mutex::new(Ok(())),
        timeline_result: Mutex::new(Ok(vec![])),
        seen_request_ids: Mutex::new(vec![]),
        counters: CallCounters::default(),
    });
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock).await,
        AdapterMetrics::for_test(),
    );
    let err = adapter
        .create_execution(CreateExecutionReq {
            journey_version: "bogus".into(),
            org_id: "org-1".into(),
            engagement_id: EngagementId::default(),
        })
        .await
        .expect_err("fail");
    assert!(matches!(err, JmError::Permanent(_)));
}

#[tokio::test]
async fn create_execution_unavailable_maps_to_unavailable() {
    let mock = Arc::new(MockJm {
        create_result: Mutex::new(Err(Status::unavailable("down"))),
        cancel_result: Mutex::new(Ok(())),
        timeline_result: Mutex::new(Ok(vec![])),
        seen_request_ids: Mutex::new(vec![]),
        counters: CallCounters::default(),
    });
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock).await,
        AdapterMetrics::for_test(),
    );
    let err = adapter
        .create_execution(CreateExecutionReq {
            journey_version: "v1".into(),
            org_id: "org-1".into(),
            engagement_id: EngagementId::default(),
        })
        .await
        .expect_err("fail");
    assert!(matches!(err, JmError::Unavailable));
}
```

- [ ] **Step 6: Add retry-budget test (writes = 2 attempts)**

```rust
#[tokio::test]
async fn create_execution_retries_exactly_twice_on_transient() {
    let mock = Arc::new(MockJm {
        create_result: Mutex::new(Err(Status::unavailable("flaky"))),
        cancel_result: Mutex::new(Ok(())),
        timeline_result: Mutex::new(Ok(vec![])),
        seen_request_ids: Mutex::new(vec![]),
        counters: CallCounters::default(),
    });
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let _ = adapter
        .create_execution(CreateExecutionReq {
            journey_version: "v1".into(),
            org_id: "org-1".into(),
            engagement_id: EngagementId::default(),
        })
        .await;
    assert_eq!(*mock.counters.create.lock().unwrap(), 2);
}

#[tokio::test]
async fn create_execution_request_id_is_stable_across_retries() {
    let mock = Arc::new(MockJm {
        create_result: Mutex::new(Err(Status::unavailable("flaky"))),
        cancel_result: Mutex::new(Ok(())),
        timeline_result: Mutex::new(Ok(vec![])),
        seen_request_ids: Mutex::new(vec![]),
        counters: CallCounters::default(),
    });
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let _ = adapter
        .create_execution(CreateExecutionReq {
            journey_version: "v1".into(),
            org_id: "org-1".into(),
            engagement_id: EngagementId::default(),
        })
        .await;
    let ids = mock.seen_request_ids.lock().unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1], "request_id must be stable across retries");
}
```

- [ ] **Step 7: Run all create_execution tests, expect pass**

```bash
cargo test -p engagement-hub-adapters journey_manager_grpc::tests::create_execution
```

Expected: 4 PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/engagement-hub-adapters/src/journey_manager_grpc.rs
git commit -m "journey: implement create_execution with WRITE_RETRY + request_id stamping

Refs #11"
```

---

### Task 11: `JourneyManagerGrpcAdapter::cancel_execution` (TDD)

**Files:**
- Modify: `crates/engagement-hub-adapters/src/journey_manager_grpc.rs`

- [ ] **Step 1: Write failing tests**

Append to the `tests` module:

```rust
#[tokio::test]
async fn cancel_execution_happy_path() {
    let mock = Arc::new(MockJm::always_ok_create(Uuid::new_v4()));
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    adapter
        .cancel_execution(
            &ExecutionRef::new(Uuid::new_v4()),
            CancelReason::CompensateFailedBind,
        )
        .await
        .expect("ok");
    assert_eq!(*mock.counters.cancel.lock().unwrap(), 1);
}

#[tokio::test]
async fn cancel_execution_retries_five_times_on_transient() {
    let mock = Arc::new(MockJm {
        create_result: Mutex::new(Ok(ExecutionRefProto { id: Uuid::new_v4().to_string() })),
        cancel_result: Mutex::new(Err(Status::unavailable("flaky"))),
        timeline_result: Mutex::new(Ok(vec![])),
        seen_request_ids: Mutex::new(vec![]),
        counters: CallCounters::default(),
    });
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let _ = adapter
        .cancel_execution(
            &ExecutionRef::new(Uuid::new_v4()),
            CancelReason::CompensateFailedBind,
        )
        .await;
    assert_eq!(*mock.counters.cancel.lock().unwrap(), 5);
}
```

- [ ] **Step 2: Replace `cancel_execution`'s `unimplemented!()` with real impl**

```rust
async fn cancel_execution(
    &self,
    ref_: &ExecutionRef,
    reason: CancelReason,
) -> Result<(), JmError> {
    let client = self.client.clone();
    let metrics = self.metrics.clone();
    let request_id = Uuid::new_v4().to_string();
    tracing::Span::current().record("adapter.request_id", request_id.as_str());
    let ref_id = ref_.as_uuid().to_string();
    let reason_proto = cancel_reason_to_proto(reason);

    with_retry(CLEANUP_RETRY, None, "journey_manager", Some(&metrics), move || {
        let mut c = client.clone();
        let r = proto::CancelExecutionRequest {
            request_id: request_id.clone(),
            execution_ref: Some(proto::ExecutionRefProto { id: ref_id.clone() }),
            reason: reason_proto as i32,
        };
        async move {
            c.cancel_execution(r).await.map_err(map_status).map(|_| ())
        }
    })
    .await
}
```

- [ ] **Step 3: Run, expect pass**

```bash
cargo test -p engagement-hub-adapters journey_manager_grpc::tests::cancel_execution
```

Expected: 2 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/src/journey_manager_grpc.rs
git commit -m "journey: implement cancel_execution with CLEANUP_RETRY (5 attempts)

Refs #11"
```

---

### Task 12: `JourneyManagerGrpcAdapter::get_execution_timeline` (TDD)

**Files:**
- Modify: `crates/engagement-hub-adapters/src/journey_manager_grpc.rs`

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test]
async fn get_execution_timeline_returns_events_in_order() {
    let mock = Arc::new(MockJm {
        create_result: Mutex::new(Err(Status::not_found("n/a"))),
        cancel_result: Mutex::new(Ok(())),
        timeline_result: Mutex::new(Ok(vec![
            proto::TimelineEventProto { sequence: 1, kind: "node_entered".into() },
            proto::TimelineEventProto { sequence: 2, kind: "node_exited".into() },
        ])),
        seen_request_ids: Mutex::new(vec![]),
        counters: CallCounters::default(),
    });
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock).await,
        AdapterMetrics::for_test(),
    );
    let t = adapter
        .get_execution_timeline(
            &ExecutionRef::new(Uuid::new_v4()),
            TimelineOpts { after_sequence: None },
        )
        .await
        .expect("ok");
    assert_eq!(t.events.len(), 2);
    assert_eq!(t.events[0].sequence, 1);
    assert_eq!(t.events[0].kind, "node_entered");
}
```

- [ ] **Step 2: Replace `get_execution_timeline`'s `unimplemented!()` with real impl**

```rust
async fn get_execution_timeline(
    &self,
    ref_: &ExecutionRef,
    opts: TimelineOpts,
) -> Result<Timeline, JmError> {
    let client = self.client.clone();
    let metrics = self.metrics.clone();
    let request_id = Uuid::new_v4().to_string();
    tracing::Span::current().record("adapter.request_id", request_id.as_str());
    let ref_id = ref_.as_uuid().to_string();
    let after = opts.after_sequence;

    with_retry(DEFAULT_RETRY, None, "journey_manager", Some(&metrics), move || {
        let mut c = client.clone();
        let r = proto::GetExecutionTimelineRequest {
            request_id: request_id.clone(),
            execution_ref: Some(proto::ExecutionRefProto { id: ref_id.clone() }),
            after_sequence: after,
        };
        async move {
            c.get_execution_timeline(r)
                .await
                .map_err(map_status)
                .map(|resp| {
                    let events = resp
                        .into_inner()
                        .events
                        .into_iter()
                        .map(|e| TimelineEvent {
                            sequence: e.sequence,
                            kind: e.kind,
                        })
                        .collect();
                    Timeline { events }
                })
        }
    })
    .await
}
```

- [ ] **Step 3: Run, expect pass**

```bash
cargo test -p engagement-hub-adapters journey_manager_grpc::tests::get_execution_timeline
```

Expected: 1 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/src/journey_manager_grpc.rs
git commit -m "journey: implement get_execution_timeline with DEFAULT_RETRY (3 attempts)

Refs #11"
```

---

### Task 13: JM cross-cutting tests — panic, deadline, cross-call request_id

**Files:**
- Modify: `crates/engagement-hub-adapters/src/journey_manager_grpc.rs`

- [ ] **Step 1: Add cross-call request_id distinctness test**

```rust
#[tokio::test]
async fn cross_call_request_ids_are_distinct() {
    // Two consecutive calls on the same adapter instance must stamp different request_ids.
    let mock = Arc::new(MockJm::always_ok_create(Uuid::new_v4()));
    let adapter = JourneyManagerGrpcAdapter::new(
        start_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let _ = adapter
        .create_execution(CreateExecutionReq {
            journey_version: "v1".into(),
            org_id: "org-1".into(),
            engagement_id: EngagementId::default(),
        })
        .await;
    let _ = adapter
        .cancel_execution(
            &ExecutionRef::new(Uuid::new_v4()),
            CancelReason::CompensateFailedBind,
        )
        .await;
    let ids = mock.seen_request_ids.lock().unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1], "request_id must NOT be reused across method invocations");
}
```

- [ ] **Step 2: Add timeout-fires test (slow mock)**

```rust
struct SlowMockJm;

#[tonic::async_trait]
impl JmServer for SlowMockJm {
    async fn create_execution(
        &self,
        _: Request<CreateExecutionRequest>,
    ) -> Result<Response<CreateExecutionResponse>, Status> {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        Ok(Response::new(CreateExecutionResponse {
            execution_ref: Some(ExecutionRefProto { id: Uuid::new_v4().to_string() }),
        }))
    }
    async fn cancel_execution(
        &self,
        _: Request<CancelExecutionRequest>,
    ) -> Result<Response<CancelExecutionResponse>, Status> {
        unimplemented!()
    }
    async fn get_execution_timeline(
        &self,
        _: Request<GetExecutionTimelineRequest>,
    ) -> Result<Response<GetExecutionTimelineResponse>, Status> {
        unimplemented!()
    }
}

#[tokio::test]
async fn slow_downstream_does_not_hang_forever_when_caller_adds_timeout() {
    // Until T1-06 wires DeadlineContext, the adapter relies on tonic's
    // per-request timeout. Construct a Request with a short timeout via the
    // adapter's underlying channel and confirm it returns (rather than hanging
    // indefinitely).
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(JourneyManagerServer::new(SlowMockJm))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .timeout(std::time::Duration::from_millis(200))
        .connect()
        .await
        .unwrap();
    let adapter = JourneyManagerGrpcAdapter::new(channel, AdapterMetrics::for_test());
    let start = std::time::Instant::now();
    let err = adapter
        .create_execution(CreateExecutionReq {
            journey_version: "v1".into(),
            org_id: "org-1".into(),
            engagement_id: EngagementId::default(),
        })
        .await
        .expect_err("must time out, not succeed");
    // Channel-level timeout fires; this maps to a transient error (Cancelled or DeadlineExceeded).
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "did not time out: {:?}", start.elapsed());
    // Exact error variant depends on tonic version; we only assert it errored.
    let _ = err;
}
```

- [ ] **Step 3: Run all JM tests, expect pass**

```bash
cargo test -p engagement-hub-adapters journey_manager_grpc
```

Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/src/journey_manager_grpc.rs
git commit -m "journey: cross-cutting tests for request_id distinctness and timeout

Refs #11"
```

---

### Task 14: `VoiceManagerGrpcAdapter` skeleton + `map_status`

**Files:**
- Create: `crates/engagement-hub-adapters/src/voice_manager_grpc.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

- [ ] **Step 1: Create the file with skeleton**

Create `crates/engagement-hub-adapters/src/voice_manager_grpc.rs`:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::VmError,
    ports::VoiceManagerPort,
    types::{
        CreateTelephonyReq, IssueTestTokenReq, ListTelephoniesReq, StartVoiceSessionReq, StopMode,
        Telephony, TelephonyId, TestToken, UpdateTelephonyReq, VoiceSessionRef,
    },
};
use tonic::{Code, transport::Channel};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{
        CLEANUP_RETRY, DEFAULT_RETRY, RetryConfig, WRITE_RETRY, with_retry,
    },
};

mod proto {
    tonic::include_proto!("revocall.voice.v1");
}
use proto::voice_manager_client::VoiceManagerClient;

fn map_status(s: tonic::Status) -> VmError {
    match s.code() {
        Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::Cancelled => VmError::Permanent(format!("{:?}: {}", s.code(), s.message())),
        Code::Unavailable => VmError::Unavailable,
        _ => VmError::Transient(format!("{:?}: {}", s.code(), s.message())),
    }
}

fn stop_mode_to_proto(m: &StopMode) -> proto::StopMode {
    match m {
        StopMode::Abort => proto::StopMode::Abort,
        StopMode::Graceful => proto::StopMode::Graceful,
    }
}

/// 1 attempt — used for stop_voice_session(mode=Graceful), which is NOT idempotent.
const GRACEFUL_STOP_RETRY: RetryConfig = RetryConfig {
    max_attempts: 1,
    initial_backoff: std::time::Duration::from_millis(50),
    max_backoff: std::time::Duration::from_secs(2),
};

fn telephony_from_proto(t: proto::TelephonyProto) -> Result<Telephony, VmError> {
    let id = t.id.parse::<Uuid>()
        .map(TelephonyId::from)
        .map_err(|e| VmError::Permanent(format!("bad telephony id: {e}")))?;
    Ok(Telephony {
        id,
        org_id: t.org_id,
        phone_number: t.phone_number,
    })
}

pub struct VoiceManagerGrpcAdapter {
    client: VoiceManagerClient<Channel>,
    metrics: Arc<AdapterMetrics>,
}

impl VoiceManagerGrpcAdapter {
    pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client: VoiceManagerClient::new(channel),
            metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    // Populated by Tasks 15–18.
}
```

- [ ] **Step 2: Wire into `lib.rs`**

```rust
pub mod voice_manager_grpc;
pub use voice_manager_grpc::VoiceManagerGrpcAdapter;
```

- [ ] **Step 3: Build**

```bash
cargo build -p engagement-hub-adapters
```

Expected: success (trait not yet impl'd — fine).

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/src/voice_manager_grpc.rs \
        crates/engagement-hub-adapters/src/lib.rs
git commit -m "voice: scaffold VoiceManagerGrpcAdapter

Refs #11"
```

---

### Task 15: VM gRPC trait impl — voice session + token + telephony CRUD (single task because tests share mock harness)

**Files:**
- Modify: `crates/engagement-hub-adapters/src/voice_manager_grpc.rs`

This task is larger; tests and impl land together because all 8 methods share the mock server harness.

- [ ] **Step 1: Write the mock harness in `tests` module**

```rust
use super::*;
use proto::{
    voice_manager_server::{VoiceManager as VmServer, VoiceManagerServer},
    CreateTelephonyRequest, CreateTelephonyResponse, DeleteTelephonyRequest,
    DeleteTelephonyResponse, GetTelephonyRequest, GetTelephonyResponse, IssueTestTokenRequest,
    IssueTestTokenResponse, ListTelephoniesRequest, ListTelephoniesResponse,
    StartVoiceSessionRequest, StartVoiceSessionResponse, StopVoiceSessionRequest,
    StopVoiceSessionResponse, TelephonyProto, UpdateTelephonyRequest, UpdateTelephonyResponse,
    VoiceSessionRefProto,
};
use std::sync::Mutex;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status, transport::Server};

#[derive(Default)]
struct VmCounters {
    start: Mutex<u32>,
    stop: Mutex<u32>,
    token: Mutex<u32>,
    create_tel: Mutex<u32>,
    list_tel: Mutex<u32>,
    get_tel: Mutex<u32>,
    update_tel: Mutex<u32>,
    delete_tel: Mutex<u32>,
}

struct MockVm {
    start_result: Mutex<Result<VoiceSessionRefProto, Status>>,
    stop_result: Mutex<Result<(), Status>>,
    token_result: Mutex<Result<String, Status>>,
    telephony_result: Mutex<Result<TelephonyProto, Status>>,
    list_result: Mutex<Result<Vec<TelephonyProto>, Status>>,
    seen_request_ids: Mutex<Vec<String>>,
    seen_stop_modes: Mutex<Vec<i32>>,
    counters: VmCounters,
}

impl MockVm {
    fn happy(default_id: Uuid) -> Self {
        Self {
            start_result: Mutex::new(Ok(VoiceSessionRefProto { id: default_id.to_string() })),
            stop_result: Mutex::new(Ok(())),
            token_result: Mutex::new(Ok("token-abc".into())),
            telephony_result: Mutex::new(Ok(TelephonyProto {
                id: default_id.to_string(),
                org_id: "org-1".into(),
                phone_number: "+60123456789".into(),
            })),
            list_result: Mutex::new(Ok(vec![])),
            seen_request_ids: Mutex::new(vec![]),
            seen_stop_modes: Mutex::new(vec![]),
            counters: VmCounters::default(),
        }
    }
}

#[tonic::async_trait]
impl VmServer for MockVm {
    async fn start_voice_session(
        &self,
        req: Request<StartVoiceSessionRequest>,
    ) -> Result<Response<StartVoiceSessionResponse>, Status> {
        *self.counters.start.lock().unwrap() += 1;
        self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
        let r = self.start_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
        Ok(Response::new(StartVoiceSessionResponse { session_ref: Some(r) }))
    }

    async fn stop_voice_session(
        &self,
        req: Request<StopVoiceSessionRequest>,
    ) -> Result<Response<StopVoiceSessionResponse>, Status> {
        *self.counters.stop.lock().unwrap() += 1;
        let inner = req.into_inner();
        self.seen_request_ids.lock().unwrap().push(inner.request_id);
        self.seen_stop_modes.lock().unwrap().push(inner.mode);
        self.stop_result.lock().unwrap().as_ref().map(|_| ()).map_err(|e| e.clone())?;
        Ok(Response::new(StopVoiceSessionResponse {}))
    }

    async fn issue_test_token(
        &self,
        req: Request<IssueTestTokenRequest>,
    ) -> Result<Response<IssueTestTokenResponse>, Status> {
        *self.counters.token.lock().unwrap() += 1;
        self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
        let t = self.token_result.lock().unwrap().as_ref().map(|s| s.clone()).map_err(|e| e.clone())?;
        Ok(Response::new(IssueTestTokenResponse { token: t }))
    }

    async fn create_telephony(
        &self,
        req: Request<CreateTelephonyRequest>,
    ) -> Result<Response<CreateTelephonyResponse>, Status> {
        *self.counters.create_tel.lock().unwrap() += 1;
        self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
        let t = self.telephony_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
        Ok(Response::new(CreateTelephonyResponse { telephony: Some(t) }))
    }

    async fn list_telephonies(
        &self,
        req: Request<ListTelephoniesRequest>,
    ) -> Result<Response<ListTelephoniesResponse>, Status> {
        *self.counters.list_tel.lock().unwrap() += 1;
        self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
        let v = self.list_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
        Ok(Response::new(ListTelephoniesResponse { telephonies: v }))
    }

    async fn get_telephony(
        &self,
        req: Request<GetTelephonyRequest>,
    ) -> Result<Response<GetTelephonyResponse>, Status> {
        *self.counters.get_tel.lock().unwrap() += 1;
        self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
        let t = self.telephony_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
        Ok(Response::new(GetTelephonyResponse { telephony: Some(t) }))
    }

    async fn update_telephony(
        &self,
        req: Request<UpdateTelephonyRequest>,
    ) -> Result<Response<UpdateTelephonyResponse>, Status> {
        *self.counters.update_tel.lock().unwrap() += 1;
        self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
        let t = self.telephony_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
        Ok(Response::new(UpdateTelephonyResponse { telephony: Some(t) }))
    }

    async fn delete_telephony(
        &self,
        req: Request<DeleteTelephonyRequest>,
    ) -> Result<Response<DeleteTelephonyResponse>, Status> {
        *self.counters.delete_tel.lock().unwrap() += 1;
        self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
        self.stop_result.lock().unwrap().as_ref().map(|_| ()).map_err(|e| e.clone())?;
        Ok(Response::new(DeleteTelephonyResponse {}))
    }
}

async fn start_vm_server(mock: Arc<MockVm>) -> Channel {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(VoiceManagerServer::from_arc(mock))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    Channel::from_shared(format!("http://{addr}")).unwrap().connect().await.unwrap()
}
```

- [ ] **Step 2: Implement the full `VoiceManagerPort` trait**

Above the `tests` module add the trait impl:

```rust
#[async_trait]
impl VoiceManagerPort for VoiceManagerGrpcAdapter {
    async fn start_voice_session(
        &self,
        req: StartVoiceSessionReq,
    ) -> Result<VoiceSessionRef, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::StartVoiceSessionRequest {
                request_id: request_id.clone(),
                engagement_id: req.engagement_id.to_string(),
                org_id: req.org_id.clone(),
            };
            async move {
                c.start_voice_session(r).await.map_err(map_status).and_then(|resp| {
                    let sr = resp.into_inner().session_ref.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty session_ref".into())
                    })?;
                    let uid = sr.id.parse::<Uuid>().map_err(|e| {
                        VmError::Permanent(format!("bad session_ref uuid: {e}"))
                    })?;
                    Ok(VoiceSessionRef::new(uid))
                })
            }
        })
        .await
    }

    async fn stop_voice_session(
        &self,
        ref_: &VoiceSessionRef,
        mode: StopMode,
    ) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let ref_id = ref_.as_uuid().to_string();
        let mode_proto = stop_mode_to_proto(&mode);
        // Graceful is NOT idempotent — single attempt only. Abort is idempotent — 5 attempts.
        let policy = match mode {
            StopMode::Abort => CLEANUP_RETRY,
            StopMode::Graceful => GRACEFUL_STOP_RETRY,
        };

        with_retry(policy, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::StopVoiceSessionRequest {
                request_id: request_id.clone(),
                session_ref: Some(proto::VoiceSessionRefProto { id: ref_id.clone() }),
                mode: mode_proto as i32,
            };
            async move {
                c.stop_voice_session(r).await.map_err(map_status).map(|_| ())
            }
        })
        .await
    }

    async fn issue_test_token(
        &self,
        req: IssueTestTokenReq,
    ) -> Result<TestToken, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(DEFAULT_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::IssueTestTokenRequest {
                request_id: request_id.clone(),
                org_id: req.org_id.clone(),
            };
            async move {
                c.issue_test_token(r).await.map_err(map_status).map(|resp| TestToken {
                    token: resp.into_inner().token,
                })
            }
        })
        .await
    }

    async fn create_telephony(
        &self,
        req: CreateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::CreateTelephonyRequest {
                request_id: request_id.clone(),
                org_id: req.org_id.clone(),
                phone_number: req.phone_number.clone(),
            };
            async move {
                c.create_telephony(r).await.map_err(map_status).and_then(|resp| {
                    let t = resp.into_inner().telephony.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty telephony".into())
                    })?;
                    telephony_from_proto(t)
                })
            }
        })
        .await
    }

    async fn list_telephonies(
        &self,
        req: ListTelephoniesReq,
    ) -> Result<Vec<Telephony>, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(DEFAULT_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::ListTelephoniesRequest {
                request_id: request_id.clone(),
                org_id: req.org_id.clone(),
                page_token: req.page_token.clone(),
            };
            async move {
                c.list_telephonies(r).await.map_err(map_status).and_then(|resp| {
                    resp.into_inner()
                        .telephonies
                        .into_iter()
                        .map(telephony_from_proto)
                        .collect::<Result<Vec<_>, _>>()
                })
            }
        })
        .await
    }

    async fn get_telephony(
        &self,
        id: &TelephonyId,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let tid = id.as_uuid().to_string();

        with_retry(DEFAULT_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::GetTelephonyRequest {
                request_id: request_id.clone(),
                telephony_id: tid.clone(),
            };
            async move {
                c.get_telephony(r).await.map_err(map_status).and_then(|resp| {
                    let t = resp.into_inner().telephony.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty telephony".into())
                    })?;
                    telephony_from_proto(t)
                })
            }
        })
        .await
    }

    async fn update_telephony(
        &self,
        req: UpdateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let tid = req.id.as_uuid().to_string();

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::UpdateTelephonyRequest {
                request_id: request_id.clone(),
                telephony_id: tid.clone(),
                phone_number: req.phone_number.clone(),
            };
            async move {
                c.update_telephony(r).await.map_err(map_status).and_then(|resp| {
                    let t = resp.into_inner().telephony.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty telephony".into())
                    })?;
                    telephony_from_proto(t)
                })
            }
        })
        .await
    }

    async fn delete_telephony(
        &self,
        id: &TelephonyId,
        usage: &str,
    ) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let tid = id.as_uuid().to_string();
        let usage = usage.to_string();

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::DeleteTelephonyRequest {
                request_id: request_id.clone(),
                telephony_id: tid.clone(),
                usage: usage.clone(),
            };
            async move {
                c.delete_telephony(r).await.map_err(map_status).map(|_| ())
            }
        })
        .await
    }
}
```

- [ ] **Step 3: Add per-method happy-path tests**

```rust
#[tokio::test]
async fn start_voice_session_happy() {
    let sid = Uuid::new_v4();
    let mock = Arc::new(MockVm::happy(sid));
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let r = adapter.start_voice_session(StartVoiceSessionReq {
        engagement_id: EngagementId::default(),
        org_id: "org-1".into(),
    }).await.expect("ok");
    assert_eq!(r.as_uuid(), &sid);
}

#[tokio::test]
async fn stop_voice_session_abort_uses_5_attempts_on_transient() {
    let mock = Arc::new(MockVm {
        stop_result: Mutex::new(Err(Status::unavailable("flaky"))),
        ..MockVm::happy(Uuid::new_v4())
    });
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let _ = adapter.stop_voice_session(
        &VoiceSessionRef::new(Uuid::new_v4()),
        StopMode::Abort,
    ).await;
    assert_eq!(*mock.counters.stop.lock().unwrap(), 5);
}

#[tokio::test]
async fn stop_voice_session_graceful_uses_1_attempt_on_transient() {
    let mock = Arc::new(MockVm {
        stop_result: Mutex::new(Err(Status::unavailable("flaky"))),
        ..MockVm::happy(Uuid::new_v4())
    });
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let _ = adapter.stop_voice_session(
        &VoiceSessionRef::new(Uuid::new_v4()),
        StopMode::Graceful,
    ).await;
    assert_eq!(*mock.counters.stop.lock().unwrap(), 1, "Graceful must not retry");
}

#[tokio::test]
async fn stop_voice_session_passes_mode_correctly() {
    let mock = Arc::new(MockVm::happy(Uuid::new_v4()));
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    adapter.stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Abort).await.unwrap();
    adapter.stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Graceful).await.unwrap();
    let modes = mock.seen_stop_modes.lock().unwrap();
    assert_eq!(modes[0], proto::StopMode::Abort as i32);
    assert_eq!(modes[1], proto::StopMode::Graceful as i32);
}

#[tokio::test]
async fn issue_test_token_happy() {
    let mock = Arc::new(MockVm::happy(Uuid::new_v4()));
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock).await,
        AdapterMetrics::for_test(),
    );
    let t = adapter.issue_test_token(IssueTestTokenReq { org_id: "org-1".into() }).await.unwrap();
    assert_eq!(t.token, "token-abc");
}

#[tokio::test]
async fn telephony_crud_roundtrip() {
    let tid = Uuid::new_v4();
    let mock = Arc::new(MockVm::happy(tid));
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let created = adapter.create_telephony(CreateTelephonyReq {
        org_id: "org-1".into(),
        phone_number: "+60123456789".into(),
    }).await.unwrap();
    assert_eq!(created.org_id, "org-1");

    let got = adapter.get_telephony(&TelephonyId::from(tid)).await.unwrap();
    assert_eq!(got.phone_number, "+60123456789");

    let updated = adapter.update_telephony(UpdateTelephonyReq {
        id: TelephonyId::from(tid),
        phone_number: "+60111111111".into(),
    }).await.unwrap();
    assert_eq!(updated.org_id, "org-1");

    adapter.delete_telephony(&TelephonyId::from(tid), "decommissioned").await.unwrap();
    assert_eq!(*mock.counters.delete_tel.lock().unwrap(), 1);
}

#[tokio::test]
async fn list_telephonies_passes_page_token() {
    let mock = Arc::new(MockVm::happy(Uuid::new_v4()));
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock).await,
        AdapterMetrics::for_test(),
    );
    let v = adapter.list_telephonies(ListTelephoniesReq {
        org_id: "org-1".into(),
        page_token: Some("next-page".into()),
    }).await.unwrap();
    assert!(v.is_empty());
}

#[tokio::test]
async fn create_telephony_retries_twice_on_transient() {
    let mock = Arc::new(MockVm {
        telephony_result: Mutex::new(Err(Status::unavailable("flaky"))),
        ..MockVm::happy(Uuid::new_v4())
    });
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock.clone()).await,
        AdapterMetrics::for_test(),
    );
    let _ = adapter.create_telephony(CreateTelephonyReq {
        org_id: "org-1".into(),
        phone_number: "+60123456789".into(),
    }).await;
    assert_eq!(*mock.counters.create_tel.lock().unwrap(), 2);
}

#[tokio::test]
async fn issue_test_token_invalid_argument_maps_to_permanent() {
    let mock = Arc::new(MockVm {
        token_result: Mutex::new(Err(Status::invalid_argument("bad org_id"))),
        ..MockVm::happy(Uuid::new_v4())
    });
    let adapter = VoiceManagerGrpcAdapter::new(
        start_vm_server(mock).await,
        AdapterMetrics::for_test(),
    );
    let e = adapter.issue_test_token(IssueTestTokenReq { org_id: "bad".into() })
        .await.expect_err("fail");
    assert!(matches!(e, VmError::Permanent(_)));
}
```

Also add the `use` line at the top of `tests` module:

```rust
use engagement_hub_ports::types::EngagementId;
```

- [ ] **Step 4: Run all VM gRPC tests, expect pass**

```bash
cargo test -p engagement-hub-adapters voice_manager_grpc
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/engagement-hub-adapters/src/voice_manager_grpc.rs
git commit -m "voice: implement VoiceManagerGrpcAdapter full trait surface

- start_voice_session: WRITE_RETRY (2)
- stop_voice_session: branches by mode — Abort uses CLEANUP_RETRY (5),
  Graceful uses 1 attempt (non-idempotent per types.rs)
- issue_test_token: DEFAULT_RETRY (3)
- create/update/delete_telephony: WRITE_RETRY (2)
- list/get_telephony: DEFAULT_RETRY (3)

Refs #11"
```

---

### Task 16: `VoiceManagerHttpAdapter` — skeleton + helpers

**Files:**
- Create: `crates/engagement-hub-adapters/src/voice_manager_http.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

- [ ] **Step 1: Create the file**

```rust
use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::VmError,
    ports::VoiceManagerPort,
    types::{
        CreateTelephonyReq, IssueTestTokenReq, ListTelephoniesReq, StartVoiceSessionReq, StopMode,
        Telephony, TelephonyId, TestToken, UpdateTelephonyReq, VoiceSessionRef,
    },
};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{
        CLEANUP_RETRY, DEFAULT_RETRY, RetryConfig, WRITE_RETRY, with_retry,
    },
};

const GRACEFUL_STOP_RETRY: RetryConfig = RetryConfig {
    max_attempts: 1,
    initial_backoff: std::time::Duration::from_millis(50),
    max_backoff: std::time::Duration::from_secs(2),
};

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorInner>,
}

#[derive(Deserialize)]
struct ErrorInner {
    code: String,
    message: String,
}

fn map_http_status(status: StatusCode, body: &str) -> VmError {
    // Try to extract the error envelope; fall back to raw body.
    let detail = serde_json::from_str::<ErrorBody>(body)
        .ok()
        .and_then(|b| b.error)
        .map(|e| format!("{}: {}", e.code, e.message))
        .unwrap_or_else(|| body.to_string());

    match status {
        s if s.is_client_error() => VmError::Permanent(format!("{status}: {detail}")),
        StatusCode::SERVICE_UNAVAILABLE => VmError::Unavailable,
        _ => VmError::Transient(format!("{status}: {detail}")),
    }
}

#[derive(Serialize)]
struct StartVoiceSessionBody {
    engagement_id: String,
    org_id: String,
}

#[derive(Deserialize)]
struct VoiceSessionRefDto {
    id: String,
}

#[derive(Deserialize)]
struct StartVoiceSessionResp {
    session_ref: VoiceSessionRefDto,
}

#[derive(Serialize)]
struct IssueTestTokenBody {
    org_id: String,
}

#[derive(Deserialize)]
struct IssueTestTokenResp {
    token: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct TelephonyDto {
    id: String,
    org_id: String,
    phone_number: String,
}

#[derive(Deserialize)]
struct TelephonyResp {
    telephony: TelephonyDto,
}

#[derive(Deserialize)]
struct ListTelephoniesResp {
    telephonies: Vec<TelephonyDto>,
}

#[derive(Serialize)]
struct CreateTelephonyBody {
    org_id: String,
    phone_number: String,
}

#[derive(Serialize)]
struct UpdateTelephonyBody {
    phone_number: String,
}

fn telephony_from_dto(t: TelephonyDto) -> Result<Telephony, VmError> {
    let id = t
        .id
        .parse::<Uuid>()
        .map(TelephonyId::from)
        .map_err(|e| VmError::Permanent(format!("bad telephony id: {e}")))?;
    Ok(Telephony {
        id,
        org_id: t.org_id,
        phone_number: t.phone_number,
    })
}

pub struct VoiceManagerHttpAdapter {
    client: Client,
    base_url: String,
    metrics: Arc<AdapterMetrics>,
}

impl VoiceManagerHttpAdapter {
    pub fn new(client: Client, base_url: String, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client,
            base_url,
            metrics,
        }
    }

    async fn execute<F, T, B>(
        client: &Client,
        method: Method,
        url: &str,
        body: Option<&B>,
        request_id: &str,
        parse: F,
    ) -> Result<T, VmError>
    where
        F: FnOnce(reqwest::Response) -> futures::future::BoxFuture<'static, Result<T, VmError>>,
        B: Serialize + ?Sized,
    {
        let mut req = client.request(method, url).header("X-Request-Id", request_id);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| VmError::Transient(e.to_string()))?;
        if resp.status().is_success() {
            parse(resp).await
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(map_http_status(status, &body))
        }
    }
}

#[cfg(test)]
mod tests {
    // Populated by Task 17.
}
```

- [ ] **Step 2: Wire into `lib.rs`**

```rust
pub mod voice_manager_http;
pub use voice_manager_http::VoiceManagerHttpAdapter;
```

- [ ] **Step 3: Build**

```bash
cargo build -p engagement-hub-adapters
```

Expected: success.

- [ ] **Step 4: Commit**

```bash
git add crates/engagement-hub-adapters/src/voice_manager_http.rs \
        crates/engagement-hub-adapters/src/lib.rs
git commit -m "voice: scaffold VoiceManagerHttpAdapter (helpers, DTOs, status mapping)

Refs #11"
```

---

### Task 17: VM HTTP — full `VoiceManagerPort` trait impl with wiremock tests

**Files:**
- Modify: `crates/engagement-hub-adapters/src/voice_manager_http.rs`

- [ ] **Step 1: Implement the trait**

Above the `tests` module:

```rust
#[async_trait]
impl VoiceManagerPort for VoiceManagerHttpAdapter {
    async fn start_voice_session(
        &self,
        req: StartVoiceSessionReq,
    ) -> Result<VoiceSessionRef, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/voice/sessions", self.base_url);
        let body = StartVoiceSessionBody {
            engagement_id: req.engagement_id.to_string(),
            org_id: req.org_id.clone(),
        };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let b = body.clone();
            let rid = request_id.clone();
            async move {
                let mut req = c.post(&u).header("X-Request-Id", &rid).json(&b);
                let resp = req.send().await.map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let parsed: StartVoiceSessionResp = resp.json().await
                        .map_err(|e| VmError::Permanent(e.to_string()))?;
                    let uid = parsed.session_ref.id.parse::<Uuid>()
                        .map_err(|e| VmError::Permanent(format!("bad session_ref uuid: {e}")))?;
                    Ok(VoiceSessionRef::new(uid))
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }

    async fn stop_voice_session(
        &self,
        ref_: &VoiceSessionRef,
        mode: StopMode,
    ) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let mode_str = match &mode {
            StopMode::Abort => "abort",
            StopMode::Graceful => "graceful",
        };
        let url = format!("{}/v1/voice/sessions/{}?mode={}", self.base_url, ref_.as_uuid(), mode_str);
        let policy = match mode {
            StopMode::Abort => CLEANUP_RETRY,
            StopMode::Graceful => GRACEFUL_STOP_RETRY,
        };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(policy, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let rid = request_id.clone();
            async move {
                let resp = c.delete(&u).header("X-Request-Id", &rid).send().await
                    .map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    Ok(())
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }

    async fn issue_test_token(
        &self,
        req: IssueTestTokenReq,
    ) -> Result<TestToken, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/voice/test_tokens", self.base_url);
        let body = IssueTestTokenBody { org_id: req.org_id.clone() };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(DEFAULT_RETRY, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let b = body.clone();
            let rid = request_id.clone();
            async move {
                let resp = c.post(&u).header("X-Request-Id", &rid).json(&b).send().await
                    .map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let parsed: IssueTestTokenResp = resp.json().await
                        .map_err(|e| VmError::Permanent(e.to_string()))?;
                    Ok(TestToken { token: parsed.token })
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }

    async fn create_telephony(
        &self,
        req: CreateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/telephonies", self.base_url);
        let body = CreateTelephonyBody { org_id: req.org_id.clone(), phone_number: req.phone_number.clone() };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let b = body.clone();
            let rid = request_id.clone();
            async move {
                let resp = c.post(&u).header("X-Request-Id", &rid).json(&b).send().await
                    .map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let parsed: TelephonyResp = resp.json().await
                        .map_err(|e| VmError::Permanent(e.to_string()))?;
                    telephony_from_dto(parsed.telephony)
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }

    async fn list_telephonies(
        &self,
        req: ListTelephoniesReq,
    ) -> Result<Vec<Telephony>, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let mut url = format!("{}/v1/telephonies?org_id={}", self.base_url, urlencoding::encode(&req.org_id));
        if let Some(pt) = &req.page_token {
            url.push_str(&format!("&page={}", urlencoding::encode(pt)));
        }
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(DEFAULT_RETRY, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let rid = request_id.clone();
            async move {
                let resp = c.get(&u).header("X-Request-Id", &rid).send().await
                    .map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let parsed: ListTelephoniesResp = resp.json().await
                        .map_err(|e| VmError::Permanent(e.to_string()))?;
                    parsed.telephonies.into_iter().map(telephony_from_dto).collect()
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }

    async fn get_telephony(
        &self,
        id: &TelephonyId,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/telephonies/{}", self.base_url, id.as_uuid());
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(DEFAULT_RETRY, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let rid = request_id.clone();
            async move {
                let resp = c.get(&u).header("X-Request-Id", &rid).send().await
                    .map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let parsed: TelephonyResp = resp.json().await
                        .map_err(|e| VmError::Permanent(e.to_string()))?;
                    telephony_from_dto(parsed.telephony)
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }

    async fn update_telephony(
        &self,
        req: UpdateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/telephonies/{}", self.base_url, req.id.as_uuid());
        let body = UpdateTelephonyBody { phone_number: req.phone_number.clone() };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let b = body.clone();
            let rid = request_id.clone();
            async move {
                let resp = c.patch(&u).header("X-Request-Id", &rid).json(&b).send().await
                    .map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let parsed: TelephonyResp = resp.json().await
                        .map_err(|e| VmError::Permanent(e.to_string()))?;
                    telephony_from_dto(parsed.telephony)
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }

    async fn delete_telephony(
        &self,
        id: &TelephonyId,
        usage: &str,
    ) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/telephonies/{}?usage={}", self.base_url, id.as_uuid(), urlencoding::encode(usage));
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager_http", Some(&metrics), move || {
            let c = client.clone();
            let u = url.clone();
            let rid = request_id.clone();
            async move {
                let resp = c.delete(&u).header("X-Request-Id", &rid).send().await
                    .map_err(|e| VmError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    Ok(())
                } else {
                    let s = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(s, &body))
                }
            }
        })
        .await
    }
}
```

Note: `urlencoding` may not yet be a dep. If `cargo build` fails on this:

```bash
cargo add urlencoding --package engagement-hub-adapters
```

Or, simpler, replace `urlencoding::encode(...)` with `percent_encoding::utf8_percent_encode(...)` if the workspace already has `percent-encoding`. Quickest: do `cargo add urlencoding -p engagement-hub-adapters`.

- [ ] **Step 2: Add `urlencoding` dep if needed**

```bash
cargo add urlencoding -p engagement-hub-adapters
```

- [ ] **Step 3: Write the wiremock tests**

In the `tests` module:

```rust
use super::*;
use engagement_hub_ports::types::EngagementId;
use serde_json::json;
use wiremock::matchers::{header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn vm(server: &MockServer) -> VoiceManagerHttpAdapter {
    VoiceManagerHttpAdapter::new(
        Client::new(),
        server.uri(),
        AdapterMetrics::for_test(),
    )
}

#[tokio::test]
async fn start_voice_session_happy() {
    let server = MockServer::start().await;
    let sid = Uuid::new_v4();
    Mock::given(method("POST"))
        .and(path("/v1/voice/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "session_ref": { "id": sid.to_string() }
        })))
        .mount(&server)
        .await;
    let a = vm(&server).await;
    let r = a.start_voice_session(StartVoiceSessionReq {
        engagement_id: EngagementId::default(),
        org_id: "org-1".into(),
    }).await.expect("ok");
    assert_eq!(r.as_uuid(), &sid);
}

#[tokio::test]
async fn start_voice_session_stamps_X_request_id_header() {
    let server = MockServer::start().await;
    let sid = Uuid::new_v4();
    Mock::given(method("POST"))
        .and(path("/v1/voice/sessions"))
        .and(header("x-request-id", wiremock::matchers::AnyMatcher))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "session_ref": { "id": sid.to_string() }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let a = vm(&server).await;
    a.start_voice_session(StartVoiceSessionReq {
        engagement_id: EngagementId::default(),
        org_id: "org-1".into(),
    }).await.expect("ok");
}

#[tokio::test]
async fn stop_voice_session_abort_retries_5_times_on_503() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/v1/voice/sessions/[0-9a-f-]+$"))
        .and(query_param("mode", "abort"))
        .respond_with(ResponseTemplate::new(503))
        .expect(5)
        .mount(&server)
        .await;
    let a = vm(&server).await;
    let e = a.stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Abort)
        .await.expect_err("fail");
    assert!(matches!(e, VmError::Unavailable));
}

#[tokio::test]
async fn stop_voice_session_graceful_attempts_only_once_on_503() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/v1/voice/sessions/[0-9a-f-]+$"))
        .and(query_param("mode", "graceful"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let a = vm(&server).await;
    let _ = a.stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Graceful).await;
}

#[tokio::test]
async fn http_4xx_maps_to_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/voice/sessions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "code": "bad_request", "message": "engagement_id required" }
        })))
        .mount(&server)
        .await;
    let a = vm(&server).await;
    let e = a.start_voice_session(StartVoiceSessionReq {
        engagement_id: EngagementId::default(),
        org_id: "org-1".into(),
    }).await.expect_err("fail");
    assert!(matches!(e, VmError::Permanent(_)));
}

#[tokio::test]
async fn http_500_maps_to_transient_and_retries_for_writes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/telephonies"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&server)
        .await;
    let a = vm(&server).await;
    let _ = a.create_telephony(CreateTelephonyReq {
        org_id: "org-1".into(),
        phone_number: "+60123456789".into(),
    }).await;
}

#[tokio::test]
async fn telephony_crud_happy() {
    let server = MockServer::start().await;
    let tid = Uuid::new_v4();
    let body = json!({
        "telephony": { "id": tid.to_string(), "org_id": "org-1", "phone_number": "+60123456789" }
    });

    Mock::given(method("POST")).and(path("/v1/telephonies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body)).mount(&server).await;
    Mock::given(method("GET")).and(path_regex(r"^/v1/telephonies/[0-9a-f-]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body)).mount(&server).await;
    Mock::given(method("PATCH")).and(path_regex(r"^/v1/telephonies/[0-9a-f-]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body)).mount(&server).await;
    Mock::given(method("DELETE")).and(path_regex(r"^/v1/telephonies/[0-9a-f-]+$"))
        .respond_with(ResponseTemplate::new(204)).mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/telephonies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"telephonies": []}))).mount(&server).await;

    let a = vm(&server).await;
    let t = a.create_telephony(CreateTelephonyReq {
        org_id: "org-1".into(), phone_number: "+60123456789".into(),
    }).await.unwrap();
    assert_eq!(t.id, TelephonyId::from(tid));

    let got = a.get_telephony(&TelephonyId::from(tid)).await.unwrap();
    assert_eq!(got.org_id, "org-1");

    let updated = a.update_telephony(UpdateTelephonyReq {
        id: TelephonyId::from(tid), phone_number: "+60111111111".into(),
    }).await.unwrap();
    assert_eq!(updated.id, TelephonyId::from(tid));

    a.delete_telephony(&TelephonyId::from(tid), "decommissioned").await.unwrap();

    let list = a.list_telephonies(ListTelephoniesReq {
        org_id: "org-1".into(), page_token: None,
    }).await.unwrap();
    assert!(list.is_empty());
}
```

- [ ] **Step 4: Run all VM HTTP tests, expect pass**

```bash
cargo test -p engagement-hub-adapters voice_manager_http
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/engagement-hub-adapters/src/voice_manager_http.rs \
        crates/engagement-hub-adapters/Cargo.toml \
        crates/engagement-hub-adapters/Cargo.lock 2>/dev/null || true
git commit -m "voice: implement VoiceManagerHttpAdapter full trait surface

Eight endpoints under /v1/voice/* and /v1/telephonies/* with the same
retry policy table as the gRPC variant. Adds urlencoding dep for path
parameter encoding.

Refs #11"
```

(The `2>/dev/null || true` on Cargo.lock allows the commit to succeed even if Cargo.lock isn't under version control in this workspace.)

---

### Task 18: Final smoke — run full workspace test + clippy + fmt

**Files:** (no edits — verification only)

- [ ] **Step 1: Full workspace test**

```bash
cargo test --workspace
```

Expected: all tests PASS across `engagement-hub-ports`, `engagement-hub-adapters`, and `engagement-hub` crates.

- [ ] **Step 2: Clippy with warnings-as-errors**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: zero warnings.

- [ ] **Step 3: Format check**

```bash
cargo fmt --all -- --check
```

Expected: zero diffs. If diffs exist, run `cargo fmt --all` and commit:

```bash
git add -u
git commit -m "fmt: cargo fmt

Refs #11"
```

- [ ] **Step 4: Cross-cutting markdown lint (story doc has lists + fences; CI runs markdownlint per memory)**

```bash
npx markdownlint-cli2 "docs/stories/T1-04-write-adapters-voice-manager-journey-manager.md"
```

Expected: zero errors. If errors: `npx markdownlint-cli2 --fix <file>` then commit.

### Deferred (explicit non-goals)

- **Deadline plumbing from inbound request through to adapter** — T1-04 ships the `with_retry` deadline arg as `Option<&DeadlineContext>` and threads `None` from every adapter method. T1-06 will replace those `None` with `Some(&DeadlineContext::from_remaining(...))` derived from the inbound gRPC `Deadline`.
- **Saga counter increment + span event emission** — T1-04 ships registration, zero-init, and the typed helper. The actual increments and span events come from T1-06 (`StartEngagement` orchestrator) and T1-07 (compensation path).
- **SDK-supplied request_id correlation** — T1-04's adapter-minted request_id is not stored in `engagement_audit` rows. If correlation becomes useful, the orchestrator can stamp it via tracing baggage in a follow-up.
- **`Retry-After` HTTP header propagation** — PRD §12 mentions this end-to-end; T1-04 does not yet honour `Retry-After` from VM HTTP 503 responses. Tracked as a follow-up.

