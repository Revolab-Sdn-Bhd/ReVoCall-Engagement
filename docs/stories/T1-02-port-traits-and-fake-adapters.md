# T1-02: Port traits + fake adapters

**Issue:** #9 | **Branch:** feat/9-port-traits | **Date:** 2026-05-15

## Brainstorm

### Problem

EH's RPC handlers need to call downstream services (Registry, JourneyManager, VoiceManager, PostCall, Analytics) without coupling test code to live infrastructure. Every downstream story — orchestrator (T1-06), saga compensation (T1-07), reconciler (T1-08), control RPCs (T1-11) — needs to component-test against these integrations cheaply.

### Options considered

**Option A — Proto-generated types in port signatures**
Use `tonic`/`prost`-generated types directly in the trait signatures. Adapters are trivially thin.
- Pro: No translation layer between port and adapter.
- Con: All consumers take a hard dependency on the proto crate; changing proto breaks every test; can't compile without the proto build step.

**Option B — Rust-native types in port signatures (chosen)**
Define plain Rust structs/enums for every request/response type. Adapters (T1-03+) own the translation to proto internally.
- Pro: Consumers depend only on `engagement-hub-ports`; no proto build step in tests; types can evolve independently of proto churn.
- Con: Translation layer in each adapter (one-time cost per adapter, not per consumer).

**Option C — `dyn Any` / type-erased ports**
Fully dynamic dispatch with no typed signatures.
- Rejected immediately: no compile-time safety, hostile DX.

### Decision

**Option B — Rust-native types.** Translation lives in the adapter crate (T1-03+), not in every consumer. Port traits are the stable API; proto is an implementation detail of each adapter.

## Implementation plan

### Design decisions locked in

- **Rust-native types in port signatures** — plain `#[derive(Debug, Clone)]` structs, no proto dependency in ports crate. Adapters reconcile to proto in T1-03+.
- **Error taxonomy: `Transient / Permanent / Unavailable`** — uniform across all 5 ports. Callers decide retry / surface / circuit-break. `#[source]` chaining deferred to T1-03 when real adapters wrap transport errors.
- **`fake` cargo feature gate** — `[features] fake = []` in ports Cargo.toml; `pub mod fake` behind `#[cfg(feature = "fake")]`. Downstream test crates opt-in; production builds never compile fakes.
- **ID newtypes** — private `Uuid` field, `new()` / `as_uuid()` / `into_uuid()`, `Display`, `Ord`. `ExecutionRef` / `VoiceSessionRef` follow the same pattern (opaque handles, not raw UUIDs).
- **`CancelReason::AdminCancelled`** added alongside PRD-specified `CompensateFailedBind` / `UserRequested` / `OrchestratorTimeout` — anticipated T1-06/T1-07 need.

### Tasks

1. **Cargo.toml setup** — add `async-trait`, `thiserror`, `uuid`, `serde` to ports; ports path dep + `reqwest`, `tokio` to adapters
2. **Domain types** (`ports/src/types.rs`) — all request/response/value types from PRD §7
3. **Error types** (`ports/src/error.rs`) — `RegistryError`, `JmError`, `VmError`, `PostCallError`, `AnalyticsError`
4. **Port traits** (`ports/src/ports.rs`) — 5 traits, 23 methods, exact PRD §7 signatures, `#[async_trait]` + `Send + Sync`
5. **Fake adapters + tests** (`ports/src/fake/`) — `Outcome<T>` queue, `Arc<Mutex<VecDeque>>` per method, `push_*` API, 107 unit tests (success w/ payload assertions, transient, permanent, unavailable, panic, FIFO ordering, empty-queue panic)
6. **Integration test scaffolding** (`adapters/src/lib.rs` + `adapters/tests/`) — 5 stub structs, 5 `#[should_panic]` tests, wired for T1-03

### Deferred to T1-03

| Item | Reason |
|------|--------|
| `#[source]` error chaining | Let adapters pick transport error types first |
| `std::sync::Mutex` → `tokio::sync::Mutex` | Guard never crosses `.await` today; safe to migrate with T1-03 |
| `async_trait` → native AFIT | Migrate after T1-03 stabilises the API surface |
| `delete_telephony usage: &str` → typed enum | PRD §7 specifies `&str`; discuss during T1-03 proto reconciliation |
