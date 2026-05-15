# T1-03: Read adapters — Registry + PostCall + Analytics

**Issue:** #10 | **Branch:** feat/10-read-adapters | **Date:** 2026-05-15

## Brainstorm

### Problem

T1-02 shipped port traits and fakes. T1-03 ships the concrete read-side adapter implementations — the three ports (Registry, PostCall, Analytics) that the orchestrator and admin-BFF will actually call at runtime. Because the Registry Service is a sibling PRD that may not be ready when EH goes to prod, we also need a fixture-backed `RegistryStubAdapter` that satisfies the Track 0 idle-mode deployment.

Cross-cutting reliability policies (retry, deadline propagation, panic safety, typed error mapping) are baked into adapter stories (PRD §12) rather than a standalone ticket, so T1-03 is also where those policies ship and get tested.

### Options considered

**Cross-cutting policy placement:**
Three options evaluated — inline per-method, shared `policy` module, or decorator wrapper types. Inline duplicates ~600 lines of policy across 3 adapters (and again in T1-04). Decorators add type complexity for minimal gain. **Shared `policy` module** (retry_call, DeadlineContext, run_safe) chosen: ~80 lines of shared code, adapters stay thin, T1-04 gets it for free.

**Registry proto sourcing:**
The PRD calls for `registry_v1::RegistryClient<Channel>` but no Registry proto exists yet. Options: copy from sibling repo, use buf.build, define forward contract here, or skip gRPC adapter. **Define forward contract** in `proto/revocall/registry/v1/service.proto`: two RPCs matching the port trait. Reconciled with Registry service's own proto when it ships.

**Deadline propagation in port traits:**
Could add a context/deadline parameter to every trait method or handle it at the call site. **Handled at orchestrator (T1-06) call site** via `tokio::time::timeout`, not inside the port trait signatures — avoids changing the T1-02 API and keeps adapters simpler. Adapters enforce their own default timeouts.

### Decision

- Define minimal Registry proto in this repo as a forward contract
- Shared `policy` module in `engagement-hub-adapters/src/policy/`
- Four adapters: `RegistryStubAdapter`, `RegistryGrpcAdapter`, `PostCallHttpAdapter`, `AnalyticsHttpAdapter`
- `InternalPanic` variant added to all 5 error enums now (T1-04 write adapters won't need a ports change)
- Deadline propagation at orchestrator call-site; adapters use built-in default timeouts
- Metrics (`engagementhub_adapter_retries_total`, `engagementhub_deadline_exceeded_total`) emitted from policy layer; counters registered in `engagement-hub/src/metrics.rs`
- Panic linter: `#![deny(clippy::unwrap_used, clippy::expect_used)]` on adapters crate

## Implementation plan

_To be filled in by writing-plans._
