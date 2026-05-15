# T1-03: Read adapters — Registry + PostCall + Analytics (with cross-cutting policies)

**Issue:** #10 | **Branch:** feat/10-read-adapters | **Date:** 2026-05-15

## Brainstorm

### Problem

T1-02 locked the port trait signatures and fake adapters. T1-03 ships the three concrete read-side adapters (`RegistryStubAdapter`, `RegistryGrpcAdapter`, `PostCallHttpAdapter`, `AnalyticsHttpAdapter`) and the cross-cutting reliability policies (retry, timeout, panic safety, typed error mapping) that every adapter must obey. Three design questions needed resolution before implementation.

### Q1 — Registry proto source

Registry Service is a sibling PRD that may slip. `RegistryGrpcAdapter` needs a compiled `registry_v1::RegistryClient<Channel>`.

**Option A — Workspace-level `proto/` placeholder (chosen):** `proto/registry/v1/registry.proto` owned by EH, committed to the repo, header documents it as a placeholder. `build.rs` in the adapters crate compiles it via `tonic-build`. Migration path when the shared proto repo arrives: replace the directory with a gitsubmodule / `buf` fetch; `build.rs` path unchanged; adapter message-mapping may need minor updates. Policy logic (retry/deadline/panic) is not affected by the proto swap.

**Option B — Pull from RevCAF:** Registry has not yet published a canonical proto; not available.

**Option C — Defer `RegistryGrpcAdapter`:** Would leave the acceptance criterion "calls Registry service via compiled proto" unmet and leave the build plumbing untested. Rejected.

**Decision: Option A.** Placeholder proto at `proto/registry/v1/registry.proto`; full gRPC adapter implemented against it now.

### Q2 — Deadline propagation

The port traits carry no context/deadline parameter (locked from T1-02). PRD §12 requires `adapter_deadline = min(caller_remaining - 50ms, adapter_default)`.

**Option A — Adapter-default timeout only (chosen):** each adapter struct holds `timeout: Duration` set at construction; every call wraps with `tokio::time::timeout(self.timeout, …)`. No cross-crate changes. True caller-deadline threading deferred to T1-06.

**Option B — `deadline` field in request types:** additive change to T1-02 types (`deadline: Option<Instant>`). Cleaner long-term; deferred until T1-06 orchestrator wires things together.

**Option C — `WithDeadline<A>` wrapper:** per-request adapter newtype; over-engineered for 3 adapters now.

**Decision: Option A.** Option B is the documented upgrade path, deferred to T1-06.

### Q3 — Cross-cutting policy structure

**Option A — Inline per method:** ~14 copies of retry + catch_unwind + backoff boilerplate; adding a new policy touches all 14 methods.

**Option B — Shared `policies.rs` helper (chosen):** single `with_retry<F, Fut, T, E>(config, timeout, target, f)` generic async fn. Each adapter method calls the helper. Policy changes are a one-file edit.

**Option C — `PolicyAdapter<A>` wrapping struct:** clean separation but overkill for 3 adapters; adds DI complexity in `main.rs`.

**Decision: Option B.** `with_retry` + `IsRetryable` trait is the right reuse unit here.

### Endpoint contracts

Real contracts extracted from admin-backend (`cmd/server/admin/calllog/services.go` and `analytic/service.go`).

**PostCall** (base: ai-handler / post-call-worker):
- `GET /calls/{id}/transcription`
- `GET /calls/{id}/summary` → unwrap `.data`
- `GET /calls/{id}/sentiment` → unwrap `.data`
- `GET /calls/{id}/state` (output extraction) → unwrap `.data`
- `GET /calls/{agent_id}/history-call?limit=&skip=&start_date=&end_date=&identity=&id=&batch_id=`
- `GET /calls/organizations/{org_id}?limit=&skip=&start_date=&end_date=&contact_number=&call_id=`

**Analytics** (base: ai-handler):
- `GET /calls/agents/{agent_id}/analytics?metric=&granularity=&startDate=&endDate=`
- `GET /calls/agents/{agent_id}/metrics?…` → unwrap `.data`
- `GET /calls/organizations/{org_id}/analytics?…`
- `GET /calls/organizations/{org_id}/metrics?…` → unwrap `.data`

`EngagementId` is passed as-is as the downstream call ID. Final ID alignment with post-call-worker resolves in T1-06.

### File layout decided

```
proto/registry/v1/registry.proto
crates/engagement-hub-adapters/
  build.rs
  Cargo.toml  (add: prost, futures, rand, wiremock[dev])
  src/
    lib.rs
    policies.rs          (with_retry, RetryConfig, IsRetryable)
    metrics.rs           (Prometheus counters)
    registry_stub.rs     (#[cfg(feature="registry-stub")])
    registry_grpc.rs     (RegistryGrpcAdapter + prod-idle guard)
    post_call_http.rs
    analytics_http.rs
  tests/
    policies_tests.rs
    registry_stub_tests.rs
    registry_grpc_tests.rs
    post_call_http_tests.rs
    analytics_http_tests.rs
```

Cargo feature `registry-stub = []` gates `RegistryStubAdapter`. `RegistryGrpcAdapter` always compiled. Production builds ship without `registry-stub`; the runtime prod-idle guard (`EH_ENV` + `EH_TRACK_0_IDLE_MODE` check in `validate_registry_adapter_config()`) is a second safety layer for dev/staging.

### Deferred items

- **Deadline propagation (Option B):** add `deadline: Option<Instant>` to request types when T1-06 wires the orchestrator
- **`#[source]` error chaining:** implement here when transport error types are known (deferred from T1-02)
- **`async_trait` → native AFIT migration:** after API surface stabilises
- **Registry gRPC integration smoke test:** deferred until Registry Service exists

## Implementation plan

_To be written by `writing-plans` skill._
