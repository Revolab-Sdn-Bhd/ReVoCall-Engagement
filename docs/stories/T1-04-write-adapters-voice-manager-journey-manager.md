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

_Not yet planned. The next step is to invoke `writing-plans` on this brainstorm + PRD §7/§12 to produce a step-by-step plan, which will be appended as a new `## Implementation plan` section._
