# T1-03: Read adapters — Registry + PostCall + Analytics (with cross-cutting policies)

**Issue:** #10 | **Branch:** feat/10-read-adapters | **Date:** 2026-05-15

## Brainstorm

### Problem

T1-02 locked port trait signatures and fake adapters. T1-03 ships the three concrete read-side adapters (`RegistryStubAdapter`, `RegistryGrpcAdapter`, `PostCallHttpAdapter`, `AnalyticsHttpAdapter`) and the cross-cutting reliability policies (retry, timeout, panic safety, typed error mapping) that every adapter must obey. Three design questions needed resolution before implementation.

### Q1 — Registry proto source

Registry Service is a sibling PRD that may slip. `RegistryGrpcAdapter` needs a compiled `registry_v1::RegistryClient<Channel>`.

**Option A — Workspace-level `proto/` placeholder (chosen):** `proto/registry/v1/registry.proto` owned by EH, committed to the repo, header documents it as a placeholder. `build.rs` in the adapters crate compiles it via `tonic-build`. Migration path when a shared proto repo arrives: replace the directory with a gitsubmodule / `buf` fetch; `build.rs` path unchanged; adapter message-mapping may need minor updates. Policy logic (retry/deadline/panic) is not affected by the proto swap.

**Option B — Pull from RevCAF:** Registry has not yet published a canonical proto; not available.

**Option C — Defer `RegistryGrpcAdapter`:** Would leave "calls Registry service via compiled proto" unmet and leave build plumbing untested. Rejected.

**Decision: Option A.** Placeholder proto at `proto/registry/v1/registry.proto`; full gRPC adapter implemented against it now.

### Q2 — Deadline propagation

The port traits carry no context/deadline parameter (locked from T1-02). PRD §12 requires `adapter_deadline = min(caller_remaining - 50ms, adapter_default)`.

**Option A — Adapter-default timeout only (chosen):** each adapter struct holds `timeout: Duration` set at construction; every call wraps with `tokio::time::timeout(self.timeout, …)`. No cross-crate changes. True caller-deadline threading deferred to T1-06.

**Option B — `deadline` field in request types:** additive change to T1-02 types (`deadline: Option<Instant>`). Cleaner long-term; deferred until T1-06 orchestrator wires things together.

**Option C — `WithDeadline<A>` wrapper:** per-request adapter newtype; over-engineered for 3 adapters now.

**Decision: Option A.** Option B is the documented upgrade path, deferred to T1-06.

### Q3 — Cross-cutting policy structure

**Option A — Inline per method:** ~14 copies of retry + catch_unwind + backoff boilerplate; adding a new policy touches all 14 methods.

**Option B — Shared `policies.rs` helper (chosen):** single `with_retry<F, Fut, T, E>(config, timeout, target, f)` generic async fn. Each adapter method calls the helper. Policy changes are a one-file edit. T1-04 write adapters get it for free.

**Option C — `PolicyAdapter<A>` wrapping struct:** clean separation but overkill for 3 adapters; adds DI complexity in `main.rs`.

**Decision: Option B.**

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
    registry_grpc.rs     (RegistryGrpcAdapter + prod-idle guard fn)
    post_call_http.rs
    analytics_http.rs
  tests/
    policies_tests.rs
    registry_stub_tests.rs
    registry_grpc_tests.rs
    post_call_http_tests.rs
    analytics_http_tests.rs
```

Cargo feature `registry-stub = []` gates `RegistryStubAdapter`. `RegistryGrpcAdapter` always compiled. Production builds ship without `registry-stub`; the runtime prod-idle guard (`validate_registry_adapter_config()` in `registry_grpc.rs`) is a second safety layer for dev/staging.

Panic linter: CI grep in `ci-code-quality.yml` scans `crates/engagement-hub-adapters/src/` for `.unwrap()`/`.expect()` outside of `policies.rs`, failing the build if any are found.

### Error type additions (ports crate change in T1-03)

T1-02 deferred `#[source]` error chaining and additional variants to T1-03. This story adds to **all 5 error enums** in `engagement-hub-ports/src/error.rs`:

- `InternalPanic` variant — PRD §7 requires `catch_unwind → AdapterError::InternalPanic`; distinct from `Permanent` so callers can distinguish transport-permanent from adapter-panicked
- `#[source]` chaining on `Transient` and `Permanent` — wraps the underlying transport error (`tonic::Status`, `reqwest::Error`, etc.)

The ports crate is a separate crate from adapters; this is an intentional T1-03 touch on both crates.

### Deferred items

- **Deadline propagation (Option B):** add `deadline: Option<Instant>` to request types when T1-06 wires the orchestrator
- **`#[source]` error chaining:** deferred from T1-02; implement here for transport error wrapping
- **`async_trait` → native AFIT migration:** after API surface stabilises
- **Registry gRPC integration smoke test:** deferred until Registry Service exists

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement four read-side concrete adapters (`RegistryStubAdapter`, `RegistryGrpcAdapter`, `PostCallHttpAdapter`, `AnalyticsHttpAdapter`) and the shared `policies.rs` reliability layer (retry, deadline, panic-safety) that all adapters and future T1-04 write adapters depend on.

**Architecture:** Port traits + fakes in `engagement-hub-ports`; concrete adapters + policy helpers in `engagement-hub-adapters`. Flat file layout (no subdirectory nesting). `RegistryStubAdapter` is behind the `registry-stub` Cargo feature. A minimal Registry proto lives at `proto/registry/v1/registry.proto`; `build.rs` compiles it via `tonic-build`. HTTP adapters use `reqwest + json`, mapping private inner response structs to port types. Panic safety uses `AssertUnwindSafe + futures::FutureExt::catch_unwind` wrapped in a single `with_retry` helper.

**Tech Stack:** Rust 2024 edition, tonic 0.12, tonic-build 0.12 (build dep), prost 0.13, reqwest 0.12 + json feature, prometheus 0.13, futures 0.3, rand 0.8, wiremock 0.6 (dev), tokio-stream 0.1 (dev), anyhow 1, tracing 0.1

---

### File Map

**`engagement-hub-ports`** (modify):

- `src/error.rs` — add `InternalPanic` to all 5 error enums; add `IsRetryable` + `FromPanic` traits
- `src/types.rs` — update all types to match real downstream contracts (real fields, serde derives)
- `src/lib.rs` — re-export new traits

**`proto/registry/v1/`** (create):

- `registry.proto` — Registry gRPC service forward contract (2 RPCs)

**`engagement-hub-adapters`** (new/modify):

- `Cargo.toml` — add tonic, prost, futures, rand, prometheus, reqwest+json; build-dep tonic-build; dev-dep wiremock, tokio-stream; feature `registry-stub`
- `build.rs` — compile registry proto with tonic-build
- `src/lib.rs` — expose all modules
- `src/metrics.rs` — `AdapterMetrics` struct
- `src/policies.rs` — `IsRetryable`, `FromPanic`, `RetryConfig`, `DeadlineContext`, `with_retry()`
- `src/registry_stub.rs` — `RegistryStubAdapter` (behind `#[cfg(feature = "registry-stub")]`)
- `src/registry_grpc.rs` — `RegistryGrpcAdapter` + `validate_registry_adapter_config()`
- `src/post_call_http.rs` — `PostCallHttpAdapter` (6 methods, real endpoints)
- `src/analytics_http.rs` — `AnalyticsHttpAdapter` (4 methods, real endpoints)
- `tests/policies_tests.rs` — retry + deadline + panic-safety integration tests
- `tests/registry_stub_tests.rs` — stub fixture tests
- `tests/registry_grpc_tests.rs` — in-process tonic server tests
- `tests/post_call_http_tests.rs` — wiremock HTTP tests
- `tests/analytics_http_tests.rs` — wiremock HTTP tests
- `tests/adapter_scaffolding.rs` — remove the old panicking stubs

---

### Task 1: `InternalPanic`, `IsRetryable`, `FromPanic` — ports crate

**Files:**

- Modify: `crates/engagement-hub-ports/src/error.rs`
- Modify: `crates/engagement-hub-ports/src/lib.rs`

- [ ] **Step 1.1: Add `InternalPanic` variant and policy traits to `error.rs`**

  Replace the entire file:

  ```rust
  /// Implemented by error types whose variants can be retried.
  pub trait IsRetryable {
      fn is_retryable(&self) -> bool;
  }

  /// Implemented by error types that can represent a caught adapter panic.
  pub trait FromPanic {
      fn from_panic() -> Self;
  }

  #[derive(Debug, thiserror::Error)]
  pub enum RegistryError {
      #[error("transient: {0}")]
      Transient(String),
      #[error("permanent: {0}")]
      Permanent(String),
      #[error("unavailable")]
      Unavailable,
      #[error("internal panic in adapter")]
      InternalPanic,
  }

  impl IsRetryable for RegistryError {
      fn is_retryable(&self) -> bool {
          matches!(self, Self::Transient(_) | Self::Unavailable)
      }
  }
  impl FromPanic for RegistryError { fn from_panic() -> Self { Self::InternalPanic } }

  #[derive(Debug, thiserror::Error)]
  pub enum JmError {
      #[error("transient: {0}")]
      Transient(String),
      #[error("permanent: {0}")]
      Permanent(String),
      #[error("unavailable")]
      Unavailable,
      #[error("internal panic in adapter")]
      InternalPanic,
  }

  impl IsRetryable for JmError {
      fn is_retryable(&self) -> bool {
          matches!(self, Self::Transient(_) | Self::Unavailable)
      }
  }
  impl FromPanic for JmError { fn from_panic() -> Self { Self::InternalPanic } }

  #[derive(Debug, thiserror::Error)]
  pub enum VmError {
      #[error("transient: {0}")]
      Transient(String),
      #[error("permanent: {0}")]
      Permanent(String),
      #[error("unavailable")]
      Unavailable,
      #[error("internal panic in adapter")]
      InternalPanic,
  }

  impl IsRetryable for VmError {
      fn is_retryable(&self) -> bool {
          matches!(self, Self::Transient(_) | Self::Unavailable)
      }
  }
  impl FromPanic for VmError { fn from_panic() -> Self { Self::InternalPanic } }

  #[derive(Debug, thiserror::Error)]
  pub enum PostCallError {
      #[error("transient: {0}")]
      Transient(String),
      #[error("permanent: {0}")]
      Permanent(String),
      #[error("unavailable")]
      Unavailable,
      #[error("internal panic in adapter")]
      InternalPanic,
  }

  impl IsRetryable for PostCallError {
      fn is_retryable(&self) -> bool {
          matches!(self, Self::Transient(_) | Self::Unavailable)
      }
  }
  impl FromPanic for PostCallError { fn from_panic() -> Self { Self::InternalPanic } }

  #[derive(Debug, thiserror::Error)]
  pub enum AnalyticsError {
      #[error("transient: {0}")]
      Transient(String),
      #[error("permanent: {0}")]
      Permanent(String),
      #[error("unavailable")]
      Unavailable,
      #[error("internal panic in adapter")]
      InternalPanic,
  }

  impl IsRetryable for AnalyticsError {
      fn is_retryable(&self) -> bool {
          matches!(self, Self::Transient(_) | Self::Unavailable)
      }
  }
  impl FromPanic for AnalyticsError { fn from_panic() -> Self { Self::InternalPanic } }
  ```

- [ ] **Step 1.2: Re-export the new traits from `src/lib.rs`**

  In `crates/engagement-hub-ports/src/lib.rs`, add:

  ```rust
  pub use error::{FromPanic, IsRetryable};
  ```

  Current `lib.rs` likely has `pub mod error; pub mod ports; pub mod types;` — add the re-export line after the `pub mod error;` line.

- [ ] **Step 1.3: Compile and fix any fallout in fakes**

  ```bash
  cargo test -p engagement-hub-ports --features fake
  ```

  The fakes use exhaustive `match` arms. Since new variants were added to errors without new `Outcome` variants (the `Outcome::Panic` arm causes a panic, not an `InternalPanic` error), no fake changes are needed. Verify all tests pass.

- [ ] **Step 1.4: Commit**

  ```bash
  git add crates/engagement-hub-ports/src/error.rs crates/engagement-hub-ports/src/lib.rs
  git commit -m "feat(ports): add InternalPanic + IsRetryable + FromPanic to error enums"
  ```

---

### Task 2: Expand domain types to match real downstream contracts

**Files:**

- Modify: `crates/engagement-hub-ports/src/types.rs`
- Modify: `crates/engagement-hub-ports/Cargo.toml` (add `serde_json`)

The current types are empty structs. Replace them with fields that match the actual PostCall and Analytics APIs extracted from `admin/backendv2`. The fake adapters in `engagement-hub-ports/src/fake/` use these types as `Success(Type {})` fixtures — update those too after changing the types.

- [ ] **Step 2.1: Replace `types.rs` with field-complete types**

  ```rust
  use std::collections::HashMap;
  use uuid::Uuid;
  use serde::{Deserialize, Serialize};

  // ---------------------------------------------------------------------------
  // ID newtypes
  // ---------------------------------------------------------------------------

  #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
  pub struct EngagementId(Uuid);

  impl EngagementId {
      pub fn new() -> Self { Self(Uuid::new_v4()) }
      pub fn into_uuid(self) -> Uuid { self.0 }
      pub fn as_uuid(&self) -> &Uuid { &self.0 }
  }
  impl Default for EngagementId { fn default() -> Self { Self::new() } }
  impl From<Uuid> for EngagementId { fn from(id: Uuid) -> Self { Self(id) } }
  impl std::fmt::Display for EngagementId {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
  }

  #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
  pub struct VoiceProfileId(Uuid);

  impl VoiceProfileId {
      pub fn new() -> Self { Self(Uuid::new_v4()) }
      pub fn into_uuid(self) -> Uuid { self.0 }
      pub fn as_uuid(&self) -> &Uuid { &self.0 }
  }
  impl Default for VoiceProfileId { fn default() -> Self { Self::new() } }
  impl From<Uuid> for VoiceProfileId { fn from(id: Uuid) -> Self { Self(id) } }
  impl std::fmt::Display for VoiceProfileId {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
  }

  #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
  pub struct TelephonyId(Uuid);

  impl TelephonyId {
      pub fn new() -> Self { Self(Uuid::new_v4()) }
      pub fn into_uuid(self) -> Uuid { self.0 }
      pub fn as_uuid(&self) -> &Uuid { &self.0 }
  }
  impl Default for TelephonyId { fn default() -> Self { Self::new() } }
  impl From<Uuid> for TelephonyId { fn from(id: Uuid) -> Self { Self(id) } }
  impl std::fmt::Display for TelephonyId {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
  }

  // ---------------------------------------------------------------------------
  // RegistryPort types
  // ---------------------------------------------------------------------------

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ResolveSnapshotReq {
      pub org_id: String,
      pub journey_version: String,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ResolvedSnapshot {
      pub snapshot_id: String,
      pub journey_version: String,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct VoiceProfile {
      pub id: VoiceProfileId,
      pub name: String,
  }

  // ---------------------------------------------------------------------------
  // JourneyManagerPort types
  // ---------------------------------------------------------------------------

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct CreateExecutionReq {
      pub journey_version: String,
      pub org_id: String,
      pub engagement_id: EngagementId,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ExecutionRef { id: Uuid }
  impl ExecutionRef {
      pub fn new(id: Uuid) -> Self { Self { id } }
      pub fn as_uuid(&self) -> &Uuid { &self.id }
      pub fn into_uuid(self) -> Uuid { self.id }
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub enum CancelReason {
      CompensateFailedBind,
      UserRequested,
      OrchestratorTimeout,
      AdminCancelled,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct TimelineOpts { pub after_sequence: Option<u64> }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Timeline { pub events: Vec<TimelineEvent> }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct TimelineEvent { pub sequence: u64, pub kind: String }

  // ---------------------------------------------------------------------------
  // VoiceManagerPort types
  // ---------------------------------------------------------------------------

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct StartVoiceSessionReq { pub engagement_id: EngagementId, pub org_id: String }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct VoiceSessionRef { id: Uuid }
  impl VoiceSessionRef {
      pub fn new(id: Uuid) -> Self { Self { id } }
      pub fn as_uuid(&self) -> &Uuid { &self.id }
      pub fn into_uuid(self) -> Uuid { self.id }
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub enum StopMode { Abort, Graceful }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct IssueTestTokenReq { pub org_id: String }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct TestToken { pub token: String }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct CreateTelephonyReq { pub org_id: String, pub phone_number: String }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Telephony { pub id: TelephonyId, pub org_id: String, pub phone_number: String }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ListTelephoniesReq { pub org_id: String, pub page_token: Option<String> }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct UpdateTelephonyReq { pub id: TelephonyId, pub phone_number: String }

  // ---------------------------------------------------------------------------
  // PostCallPort types
  // Shapes match admin-backend/cmd/server/admin/calllog/types.go
  // ---------------------------------------------------------------------------

  /// Structured transcription. Adapter concatenates messages into `text` for
  /// callers that need a flat string; the full list is in `messages`.
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Transcript {
      pub messages: Vec<TranscriptMessage>,
      pub total_size: i32,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct TranscriptMessage {
      pub message: String,
      pub role: String,
      pub audio_url: Option<String>,
      pub emotion: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Summary {
      pub summary: String,
      pub resolution: Option<String>,
      pub resolution_explanation: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Sentiment {
      pub label: String,         // "positive" | "negative" | "neutral"
      pub justification: String,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct OutputExtraction {
      pub fields: Vec<OutputField>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct OutputField {
      pub key: String,
      pub value: String,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ListAgentCallLogsReq {
      pub agent_id: String,
      pub skip: Option<u32>,
      pub limit: Option<u32>,
      pub start_date: Option<String>,
      pub end_date: Option<String>,
      pub identity: Option<String>,
      pub id: Option<String>,
      pub batch_id: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ListOrgCallLogsReq {
      pub org_id: String,
      pub skip: Option<u32>,
      pub limit: Option<u32>,
      pub start_date: Option<String>,
      pub end_date: Option<String>,
      pub contact_number: Option<String>,
      pub call_id: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct CallLog {
      pub id: String,
      pub room_name: Option<String>,
      pub batch_id: Option<String>,
      pub duration: Option<i32>,
      pub identity: Option<String>,
      pub created_at: String,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Page<T> {
      pub items: Vec<T>,
      pub total_size: Option<u32>,
      pub next_page_token: Option<String>,
  }

  // ---------------------------------------------------------------------------
  // AnalyticsPort types
  // Shapes match admin-backend/cmd/server/admin/analytic/types.go
  // ---------------------------------------------------------------------------

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct GetAgentAnalyticsReq {
      pub agent_id: String,
      pub metric: Option<String>,
      pub granularity: Option<String>,
      pub start_date: Option<String>,
      pub end_date: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct GetAgentMetricsReq {
      pub agent_id: String,
      pub metric: Option<String>,
      pub granularity: Option<String>,
      pub start_date: Option<String>,
      pub end_date: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct GetOrgAnalyticsReq {
      pub org_id: String,
      pub metric: Option<String>,
      pub granularity: Option<String>,
      pub start_date: Option<String>,
      pub end_date: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct GetOrgMetricsReq {
      pub org_id: String,
      pub metric: Option<String>,
      pub granularity: Option<String>,
      pub start_date: Option<String>,
      pub end_date: Option<String>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Analytics {
      pub average_conversation_duration: f64,
      pub containment_rate: f64,
      pub customer_satisfaction_rate: f64,
      pub dropoff_rate: f64,
      pub escalation_rate: f64,
      pub total_inquiries: u32,
      pub category_counts: HashMap<String, u32>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct Metrics {
      pub categories: Vec<String>,
      pub series: Vec<f64>,
  }
  ```

- [ ] **Step 2.2: Add `serde_json` to ports Cargo.toml (needed for potential future `Value` fields)**

  In `crates/engagement-hub-ports/Cargo.toml`, under `[dependencies]`, add:

  ```toml
  serde_json = { workspace = true }
  ```

- [ ] **Step 2.3: Fix fake adapter fixture constructors**

  Run `cargo test -p engagement-hub-ports --features fake` — it will fail because the old empty-struct fixtures are now invalid. Fix each fake's test fixtures:

  In `src/fake/registry.rs`, update test fixture constructors:

  ```rust
  // Old: Outcome::Success(ResolvedSnapshot {})
  // New:
  Outcome::Success(ResolvedSnapshot { snapshot_id: "snap-1".into(), journey_version: "v1".into() })
  // Old: Outcome::Success(VoiceProfile {})
  // New:
  Outcome::Success(VoiceProfile { id: VoiceProfileId::new(), name: "test-profile".into() })
  ```

  In `src/fake/post_call.rs`, update `Transcript`, `Summary`, `Sentiment`, `OutputExtraction`, `CallLog`, `Page<CallLog>` constructors:

  ```rust
  Outcome::Success(Transcript { messages: vec![], total_size: 0 })
  Outcome::Success(Summary { summary: "test".into(), resolution: None, resolution_explanation: None })
  Outcome::Success(Sentiment { label: "neutral".into(), justification: "no data".into() })
  Outcome::Success(OutputExtraction { fields: vec![] })
  Outcome::Success(Page { items: vec![], total_size: Some(0), next_page_token: None })
  ```

  In `src/fake/analytics.rs`, update `Analytics` and `Metrics` constructors:

  ```rust
  Outcome::Success(Analytics {
      average_conversation_duration: 0.0,
      containment_rate: 0.0,
      customer_satisfaction_rate: 0.0,
      dropoff_rate: 0.0,
      escalation_rate: 0.0,
      total_inquiries: 0,
      category_counts: std::collections::HashMap::new(),
  })
  Outcome::Success(Metrics { categories: vec![], series: vec![] })
  ```

  Run `cargo test -p engagement-hub-ports --features fake` and iterate until green.

- [ ] **Step 2.4: Commit**

  ```bash
  git add crates/engagement-hub-ports/
  git commit -m "feat(ports): expand types with real downstream contract fields + serde derives"
  ```

---

### Task 3: Registry proto + build.rs

**Files:**

- Create: `proto/registry/v1/registry.proto`
- Create: `crates/engagement-hub-adapters/build.rs`
- Modify: `crates/engagement-hub-adapters/Cargo.toml`

- [ ] **Step 3.1: Create `proto/registry/v1/registry.proto`**

  ```proto
  // Forward contract for the Registry gRPC service.
  // Owned by the EH repo until Registry Service ships its canonical proto.
  // Regenerate Rust bindings: cargo build -p engagement-hub-adapters
  syntax = "proto3";

  package revocall.registry.v1;

  service Registry {
    rpc ResolveSnapshot(ResolveSnapshotRequest) returns (ResolveSnapshotResponse);
    rpc GetVoiceProfile(GetVoiceProfileRequest)  returns (GetVoiceProfileResponse);
  }

  message ResolveSnapshotRequest {
    string org_id          = 1;
    string journey_version = 2;
  }

  message ResolvedSnapshotProto {
    string snapshot_id      = 1;
    string journey_version  = 2;
  }

  message ResolveSnapshotResponse {
    ResolvedSnapshotProto snapshot = 1;
  }

  message GetVoiceProfileRequest {
    string voice_profile_id = 1;
  }

  message VoiceProfileProto {
    string id   = 1;
    string name = 2;
  }

  message GetVoiceProfileResponse {
    VoiceProfileProto profile = 1;
  }
  ```

- [ ] **Step 3.2: Update `engagement-hub-adapters/Cargo.toml`**

  Replace the file:

  ```toml
  [package]
  name = "engagement-hub-adapters"
  edition.workspace = true
  version.workspace = true
  license.workspace = true
  publish.workspace = true

  [features]
  registry-stub = []

  [dependencies]
  async-trait   = { workspace = true }
  thiserror     = { workspace = true }
  tokio         = { workspace = true }
  reqwest       = { workspace = true, features = ["json"] }
  tonic         = { workspace = true }
  prost         = { workspace = true }
  futures       = { workspace = true }
  prometheus    = { workspace = true }
  anyhow        = { workspace = true }
  tracing       = { workspace = true }
  rand          = "0.8"
  engagement-hub-ports = { path = "../engagement-hub-ports" }

  [build-dependencies]
  tonic-build = "0.12"

  [dev-dependencies]
  tokio        = { workspace = true }
  wiremock     = "0.6"
  tokio-stream = "0.1"

  [lib]
  path = "src/lib.rs"
  ```

- [ ] **Step 3.3: Create `crates/engagement-hub-adapters/build.rs`**

  ```rust
  fn main() -> Result<(), Box<dyn std::error::Error>> {
      tonic_build::configure()
          .build_server(true)
          .compile_protos(&["proto/registry/v1/registry.proto"], &["proto"])?;
      Ok(())
  }
  ```

  `build.rs` runs with the Cargo workspace root as the working directory, so `proto/registry/v1/registry.proto` resolves from `/path/to/ReVoCall-Engagement/proto/registry/v1/registry.proto`.

- [ ] **Step 3.4: Verify proto compiles**

  ```bash
  cargo build -p engagement-hub-adapters 2>&1 | grep -E "error|warning: unused" | head -20
  ```

  Expected: no errors. The generated module will be available inside `src/registry_grpc.rs` via `tonic::include_proto!("revocall.registry.v1")`.

- [ ] **Step 3.5: Commit**

  ```bash
  git add proto/registry/ crates/engagement-hub-adapters/build.rs crates/engagement-hub-adapters/Cargo.toml
  git commit -m "feat(adapters): add Registry proto + tonic-build wiring"
  ```

---

### Task 4: `AdapterMetrics` struct

**Files:**

- Create: `crates/engagement-hub-adapters/src/metrics.rs`

- [ ] **Step 4.1: Write `metrics.rs`**

  ```rust
  use std::sync::Arc;

  use anyhow::Result;
  use prometheus::{IntCounterVec, Opts, Registry};

  pub struct AdapterMetrics {
      pub retries_total: IntCounterVec,
      pub deadline_exceeded_total: IntCounterVec,
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

          Ok(Arc::new(Self { retries_total, deadline_exceeded_total }))
      }

      /// Returns a metrics instance backed by a throwaway registry (for tests).
      pub fn for_test() -> Arc<Self> {
          Self::new(&Registry::new()).expect("test metrics")
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn registers_both_counters() {
          let r = Registry::new();
          let m = AdapterMetrics::new(&r).unwrap();
          m.retries_total.with_label_values(&["registry", "1"]).inc();
          m.deadline_exceeded_total.with_label_values(&["registry"]).inc();
          use prometheus::Encoder;
          let enc = prometheus::TextEncoder::new();
          let mut buf = Vec::new();
          enc.encode(&r.gather(), &mut buf).unwrap();
          let text = String::from_utf8(buf).unwrap();
          assert!(text.contains("engagementhub_adapter_retries_total"));
          assert!(text.contains("engagementhub_deadline_exceeded_total"));
      }
  }
  ```

- [ ] **Step 4.2: Update `src/lib.rs` to expose the module**

  Replace the file:

  ```rust
  pub mod metrics;
  // remaining modules added in subsequent tasks
  ```

- [ ] **Step 4.3: Run the test**

  ```bash
  cargo test -p engagement-hub-adapters metrics
  ```

  Expected: `registers_both_counters` passes.

- [ ] **Step 4.4: Commit**

  ```bash
  git add crates/engagement-hub-adapters/src/metrics.rs crates/engagement-hub-adapters/src/lib.rs
  git commit -m "feat(adapters): add AdapterMetrics prometheus counters"
  ```

---

### Task 5: `policies.rs` — retry, deadline, panic-safety

**Files:**

- Create: `crates/engagement-hub-adapters/src/policies.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

This is the shared reliability layer. All adapters call `with_retry(config, target, metrics, f)` which applies full-jitter exponential backoff, maps panics to `InternalPanic`, and records metrics.

- [ ] **Step 5.1: Write `policies.rs`**

  ```rust
  use std::{future::Future, panic::AssertUnwindSafe, time::{Duration, Instant}};

  use futures::FutureExt;
  use rand::Rng;

  use engagement_hub_ports::error::{FromPanic, IsRetryable};

  use crate::metrics::AdapterMetrics;

  // ---------------------------------------------------------------------------
  // Retry configuration
  // ---------------------------------------------------------------------------

  #[derive(Clone, Copy, Debug)]
  pub struct RetryConfig {
      pub max_attempts: u32,
      pub initial_backoff: Duration,
      pub max_backoff: Duration,
  }

  /// 5 attempts — used for Registry.resolve_snapshot (read-only, cheap to retry).
  pub const REGISTRY_RESOLVE_RETRY: RetryConfig = RetryConfig {
      max_attempts: 5,
      initial_backoff: Duration::from_millis(50),
      max_backoff: Duration::from_secs(2),
  };

  /// 3 attempts — default for PostCall, Analytics, and Registry.get_voice_profile.
  pub const DEFAULT_RETRY: RetryConfig = RetryConfig {
      max_attempts: 3,
      initial_backoff: Duration::from_millis(50),
      max_backoff: Duration::from_secs(2),
  };

  // ---------------------------------------------------------------------------
  // Deadline
  // ---------------------------------------------------------------------------

  const PROPAGATION_MARGIN: Duration = Duration::from_millis(50);
  const ADAPTER_FLOOR: Duration = Duration::from_millis(200);

  pub struct DeadlineContext {
      deadline: Option<Instant>,
  }

  impl DeadlineContext {
      pub fn none() -> Self { Self { deadline: None } }

      /// Compute `min(remaining - 50ms, adapter_default)`.
      pub fn from_remaining(remaining: Duration, adapter_default: Duration) -> Self {
          let budget = remaining.saturating_sub(PROPAGATION_MARGIN).min(adapter_default);
          Self { deadline: Some(Instant::now() + budget) }
      }

      /// True if remaining time is below the 200ms safety floor.
      pub fn is_too_close(&self) -> bool {
          self.deadline.map_or(false, |d| {
              d.saturating_duration_since(Instant::now()) < ADAPTER_FLOOR
          })
      }
  }

  // ---------------------------------------------------------------------------
  // Core retry + panic-safety combinator
  // ---------------------------------------------------------------------------

  /// Retries `f` up to `config.max_attempts` on retryable errors, with full-jitter
  /// exponential backoff. Panics inside `f` are caught and returned as `E::from_panic()`.
  /// Retry counts are recorded to `metrics` if `Some`.
  pub async fn with_retry<F, Fut, T, E>(
      config: RetryConfig,
      target: &str,
      metrics: Option<&AdapterMetrics>,
      mut f: F,
  ) -> Result<T, E>
  where
      F: FnMut() -> Fut,
      Fut: Future<Output = Result<T, E>> + Send,
      E: IsRetryable + FromPanic + Send + 'static,
      T: Send + 'static,
  {
      let mut backoff = config.initial_backoff;
      for attempt in 0..config.max_attempts {
          let result = AssertUnwindSafe(f())
              .catch_unwind()
              .await
              .unwrap_or_else(|_| Err(E::from_panic()));

          match &result {
              Err(e) if e.is_retryable() && attempt + 1 < config.max_attempts => {
                  if let Some(m) = metrics {
                      m.retries_total
                          .with_label_values(&[target, &(attempt + 1).to_string()])
                          .inc();
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

  // ---------------------------------------------------------------------------
  // Tests
  // ---------------------------------------------------------------------------

  #[cfg(test)]
  mod tests {
      use super::*;
      use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

      #[derive(Debug, PartialEq, Clone)]
      enum E { Transient, Permanent, Panic }
      impl IsRetryable for E { fn is_retryable(&self) -> bool { matches!(self, Self::Transient) } }
      impl FromPanic for E { fn from_panic() -> Self { Self::Panic } }

      fn no_sleep_config(max: u32) -> RetryConfig {
          RetryConfig { max_attempts: max, initial_backoff: Duration::ZERO, max_backoff: Duration::ZERO }
      }

      #[tokio::test]
      async fn success_on_first_attempt() {
          let n = Arc::new(AtomicU32::new(0));
          let c = n.clone();
          let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
              let c = c.clone();
              async move { c.fetch_add(1, Ordering::SeqCst); Ok(42) }
          }).await;
          assert_eq!(r, Ok(42));
          assert_eq!(n.load(Ordering::SeqCst), 1);
      }

      #[tokio::test]
      async fn retries_transient_then_succeeds() {
          let n = Arc::new(AtomicU32::new(0));
          let c = n.clone();
          let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
              let c = c.clone();
              async move {
                  let count = c.fetch_add(1, Ordering::SeqCst);
                  if count < 2 { Err(E::Transient) } else { Ok(1) }
              }
          }).await;
          assert_eq!(r, Ok(1));
          assert_eq!(n.load(Ordering::SeqCst), 3);
      }

      #[tokio::test]
      async fn no_retry_on_permanent() {
          let n = Arc::new(AtomicU32::new(0));
          let c = n.clone();
          let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
              let c = c.clone();
              async move { c.fetch_add(1, Ordering::SeqCst); Err(E::Permanent) }
          }).await;
          assert_eq!(r, Err(E::Permanent));
          assert_eq!(n.load(Ordering::SeqCst), 1);
      }

      #[tokio::test]
      async fn exhausts_all_attempts_on_persistent_transient() {
          let n = Arc::new(AtomicU32::new(0));
          let c = n.clone();
          let r: Result<i32, E> = with_retry(no_sleep_config(3), "t", None, || {
              let c = c.clone();
              async move { c.fetch_add(1, Ordering::SeqCst); Err(E::Transient) }
          }).await;
          assert_eq!(r, Err(E::Transient));
          assert_eq!(n.load(Ordering::SeqCst), 3);
      }

      #[tokio::test]
      async fn catches_panic_and_returns_from_panic() {
          let r: Result<i32, E> = with_retry(no_sleep_config(1), "t", None, || {
              async move { panic!("adapter panic") }
          }).await;
          assert_eq!(r, Err(E::Panic));
      }

      #[tokio::test]
      async fn records_retry_metrics() {
          let m = crate::metrics::AdapterMetrics::for_test();
          let n = Arc::new(AtomicU32::new(0));
          let c = n.clone();
          let _: Result<i32, E> = with_retry(no_sleep_config(3), "reg", Some(&m), || {
              let c = c.clone();
              async move { c.fetch_add(1, Ordering::SeqCst); Err(E::Transient) }
          }).await;
          // 3 attempts → 2 retries recorded
          assert_eq!(m.retries_total.with_label_values(&["reg", "1"]).get(), 1);
          assert_eq!(m.retries_total.with_label_values(&["reg", "2"]).get(), 1);
      }

      #[test]
      fn deadline_too_close_when_remaining_less_than_200ms() {
          let ctx = DeadlineContext::from_remaining(Duration::from_millis(100), Duration::from_secs(5));
          assert!(ctx.is_too_close());
      }

      #[test]
      fn deadline_not_too_close_when_remaining_generous() {
          let ctx = DeadlineContext::from_remaining(Duration::from_secs(10), Duration::from_secs(5));
          assert!(!ctx.is_too_close());
      }

      #[test]
      fn deadline_none_never_too_close() {
          assert!(!DeadlineContext::none().is_too_close());
      }
  }
  ```

- [ ] **Step 5.2: Add `pub mod policies` to `src/lib.rs`**

  ```rust
  pub mod metrics;
  pub mod policies;
  ```

- [ ] **Step 5.3: Run tests**

  ```bash
  cargo test -p engagement-hub-adapters policies
  ```

  Expected: all 9 tests pass including `catches_panic_and_returns_from_panic`.

- [ ] **Step 5.4: Commit**

  ```bash
  git add crates/engagement-hub-adapters/src/policies.rs crates/engagement-hub-adapters/src/lib.rs
  git commit -m "feat(adapters): add policies.rs — with_retry + DeadlineContext + panic safety"
  ```

---

### Task 6: `RegistryStubAdapter`

**Files:**

- Create: `crates/engagement-hub-adapters/src/registry_stub.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

The stub is only compiled when the `registry-stub` Cargo feature is enabled. It returns from an in-memory fixture map, keyed on `journey_version`. No retries (local map), panic-safety still applied.

- [ ] **Step 6.1: Write `registry_stub.rs`**

  ```rust
  use std::collections::HashMap;

  use async_trait::async_trait;
  use engagement_hub_ports::{
      error::RegistryError,
      ports::RegistryPort,
      types::{ResolveSnapshotReq, ResolvedSnapshot, VoiceProfile, VoiceProfileId},
  };

  pub struct RegistryStubAdapter {
      snapshots: HashMap<String, ResolvedSnapshot>,
      profiles: HashMap<String, VoiceProfile>,
  }

  impl RegistryStubAdapter {
      /// Build from explicit fixture lists. Snapshot map key = `journey_version`.
      /// Profile map key = UUID string of the `VoiceProfileId`.
      pub fn new(snapshots: Vec<ResolvedSnapshot>, profiles: Vec<VoiceProfile>) -> Self {
          Self {
              snapshots: snapshots.into_iter().map(|s| (s.journey_version.clone(), s)).collect(),
              profiles: profiles.into_iter().map(|p| (p.id.as_uuid().to_string(), p)).collect(),
          }
      }

      /// Default fixtures for Track 0 idle-mode deployments.
      pub fn with_default_fixtures() -> Self {
          Self::new(
              vec![ResolvedSnapshot {
                  snapshot_id: "fixture-snap-v1".into(),
                  journey_version: "v1-fixture".into(),
              }],
              vec![],
          )
      }
  }

  #[async_trait]
  impl RegistryPort for RegistryStubAdapter {
      async fn resolve_snapshot(
          &self,
          req: ResolveSnapshotReq,
      ) -> Result<ResolvedSnapshot, RegistryError> {
          self.snapshots
              .get(&req.journey_version)
              .cloned()
              .ok_or_else(|| RegistryError::Permanent(
                  format!("stub: journey_version '{}' not found in fixtures", req.journey_version)
              ))
      }

      async fn get_voice_profile(
          &self,
          id: &VoiceProfileId,
      ) -> Result<VoiceProfile, RegistryError> {
          self.profiles
              .get(&id.as_uuid().to_string())
              .cloned()
              .ok_or_else(|| RegistryError::Permanent(
                  format!("stub: voice_profile_id '{}' not found in fixtures", id)
              ))
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use uuid::Uuid;

      fn stub() -> RegistryStubAdapter {
          RegistryStubAdapter::new(
              vec![
                  ResolvedSnapshot { snapshot_id: "snap-1".into(), journey_version: "v1".into() },
                  ResolvedSnapshot { snapshot_id: "snap-2".into(), journey_version: "v2".into() },
              ],
              vec![
                  VoiceProfile { id: VoiceProfileId::from(Uuid::nil()), name: "bot".into() },
              ],
          )
      }

      #[tokio::test]
      async fn resolve_known_version() {
          let snap = stub().resolve_snapshot(ResolveSnapshotReq {
              org_id: "org1".into(), journey_version: "v1".into(),
          }).await.expect("found");
          assert_eq!(snap.snapshot_id, "snap-1");
      }

      #[tokio::test]
      async fn resolve_unknown_version_returns_permanent() {
          let err = stub().resolve_snapshot(ResolveSnapshotReq {
              org_id: "org1".into(), journey_version: "vX".into(),
          }).await.expect_err("not found");
          assert!(matches!(err, RegistryError::Permanent(_)));
      }

      #[tokio::test]
      async fn get_known_profile() {
          let vp = stub().get_voice_profile(&VoiceProfileId::from(Uuid::nil())).await.expect("found");
          assert_eq!(vp.name, "bot");
      }

      #[tokio::test]
      async fn get_unknown_profile_returns_permanent() {
          let err = stub().get_voice_profile(&VoiceProfileId::new()).await.expect_err("not found");
          assert!(matches!(err, RegistryError::Permanent(_)));
      }

      #[tokio::test]
      async fn default_fixtures_resolves_v1_fixture() {
          let s = RegistryStubAdapter::with_default_fixtures();
          let snap = s.resolve_snapshot(ResolveSnapshotReq {
              org_id: "org1".into(), journey_version: "v1-fixture".into(),
          }).await.expect("default fixture");
          assert_eq!(snap.snapshot_id, "fixture-snap-v1");
      }
  }
  ```

- [ ] **Step 6.2: Gate behind the feature in `src/lib.rs`**

  ```rust
  pub mod metrics;
  pub mod policies;

  #[cfg(feature = "registry-stub")]
  pub mod registry_stub;
  #[cfg(feature = "registry-stub")]
  pub use registry_stub::RegistryStubAdapter;
  ```

- [ ] **Step 6.3: Run tests with the feature enabled**

  ```bash
  cargo test -p engagement-hub-adapters --features registry-stub registry_stub
  ```

  Expected: 5 tests pass.

- [ ] **Step 6.4: Commit**

  ```bash
  git add crates/engagement-hub-adapters/src/registry_stub.rs crates/engagement-hub-adapters/src/lib.rs
  git commit -m "feat(adapters): add RegistryStubAdapter behind registry-stub feature"
  ```

---

### Task 7: `RegistryGrpcAdapter`

**Files:**

- Create: `crates/engagement-hub-adapters/src/registry_grpc.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

gRPC status code mapping: `NOT_FOUND`/`INVALID_ARGUMENT`/`FAILED_PRECONDITION`/`ALREADY_EXISTS` → `Permanent`; `UNAVAILABLE` → `Unavailable`; everything else → `Transient`.

- [ ] **Step 7.1: Write `registry_grpc.rs`**

  ```rust
  use std::sync::Arc;

  use async_trait::async_trait;
  use engagement_hub_ports::{
      error::RegistryError,
      ports::RegistryPort,
      types::{ResolveSnapshotReq, ResolvedSnapshot, VoiceProfile, VoiceProfileId},
  };
  use tonic::{transport::Channel, Code};

  use crate::{
      metrics::AdapterMetrics,
      policies::{DEFAULT_RETRY, REGISTRY_RESOLVE_RETRY, with_retry},
  };

  mod proto {
      tonic::include_proto!("revocall.registry.v1");
  }
  use proto::registry_client::RegistryClient;

  fn map_status(s: tonic::Status) -> RegistryError {
      match s.code() {
          Code::NotFound | Code::InvalidArgument | Code::FailedPrecondition | Code::AlreadyExists
              => RegistryError::Permanent(s.message().to_owned()),
          Code::Unavailable
              => RegistryError::Unavailable,
          _ => RegistryError::Transient(format!("{}: {}", s.code(), s.message())),
      }
  }

  pub struct RegistryGrpcAdapter {
      client: RegistryClient<Channel>,
      metrics: Arc<AdapterMetrics>,
  }

  impl RegistryGrpcAdapter {
      pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
          Self { client: RegistryClient::new(channel), metrics }
      }
  }

  #[async_trait]
  impl RegistryPort for RegistryGrpcAdapter {
      async fn resolve_snapshot(
          &self,
          req: ResolveSnapshotReq,
      ) -> Result<ResolvedSnapshot, RegistryError> {
          let client = self.client.clone();
          let metrics = self.metrics.clone();
          with_retry(REGISTRY_RESOLVE_RETRY, "registry", Some(&metrics), move || {
              let mut c = client.clone();
              let r = proto::ResolveSnapshotRequest {
                  org_id: req.org_id.clone(),
                  journey_version: req.journey_version.clone(),
              };
              async move {
                  c.resolve_snapshot(r).await.map_err(map_status).and_then(|resp| {
                      let snap = resp.into_inner().snapshot.ok_or_else(|| {
                          RegistryError::Permanent("registry: empty snapshot in response".into())
                      })?;
                      Ok(ResolvedSnapshot {
                          snapshot_id: snap.snapshot_id,
                          journey_version: snap.journey_version,
                      })
                  })
              }
          }).await
      }

      async fn get_voice_profile(
          &self,
          id: &VoiceProfileId,
      ) -> Result<VoiceProfile, RegistryError> {
          let client = self.client.clone();
          let metrics = self.metrics.clone();
          let id_str = id.as_uuid().to_string();
          with_retry(DEFAULT_RETRY, "registry", Some(&metrics), move || {
              let mut c = client.clone();
              let req = proto::GetVoiceProfileRequest { voice_profile_id: id_str.clone() };
              async move {
                  c.get_voice_profile(req).await.map_err(map_status).and_then(|resp| {
                      let p = resp.into_inner().profile.ok_or_else(|| {
                          RegistryError::Permanent("registry: empty profile in response".into())
                      })?;
                      let uid = p.id.parse::<uuid::Uuid>()
                          .map(VoiceProfileId::from)
                          .map_err(|e| RegistryError::Permanent(format!("bad uuid: {e}")))?;
                      Ok(VoiceProfile { id: uid, name: p.name })
                  })
              }
          }).await
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      use std::sync::Mutex;
      use tokio::net::TcpListener;
      use tokio_stream::wrappers::TcpListenerStream;
      use tonic::{Request, Response, Status, transport::Server};
      use proto::{
          registry_server::{Registry, RegistryServer},
          GetVoiceProfileRequest, GetVoiceProfileResponse, ResolveSnapshotRequest,
          ResolveSnapshotResponse, ResolvedSnapshotProto, VoiceProfileProto,
      };

      struct MockRegistry {
          snap_result: Mutex<Result<ResolvedSnapshotProto, Status>>,
          profile_result: Mutex<Result<VoiceProfileProto, Status>>,
      }

      #[tonic::async_trait]
      impl Registry for MockRegistry {
          async fn resolve_snapshot(
              &self,
              _req: Request<ResolveSnapshotRequest>,
          ) -> Result<Response<ResolveSnapshotResponse>, Status> {
              let r = self.snap_result.lock().unwrap()
                  .as_ref().map(|s| s.clone()).map_err(|e| e.clone())?;
              Ok(Response::new(ResolveSnapshotResponse { snapshot: Some(r) }))
          }

          async fn get_voice_profile(
              &self,
              _req: Request<GetVoiceProfileRequest>,
          ) -> Result<Response<GetVoiceProfileResponse>, Status> {
              let r = self.profile_result.lock().unwrap()
                  .as_ref().map(|p| p.clone()).map_err(|e| e.clone())?;
              Ok(Response::new(GetVoiceProfileResponse { profile: Some(r) }))
          }
      }

      async fn start_server(mock: MockRegistry) -> Channel {
          let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
          let addr = listener.local_addr().unwrap();
          tokio::spawn(
              Server::builder()
                  .add_service(RegistryServer::new(mock))
                  .serve_with_incoming(TcpListenerStream::new(listener)),
          );
          Channel::from_shared(format!("http://{addr}")).unwrap()
              .connect().await.unwrap()
      }

      #[tokio::test]
      async fn resolve_snapshot_success() {
          let mock = MockRegistry {
              snap_result: Mutex::new(Ok(ResolvedSnapshotProto {
                  snapshot_id: "snap-grpc".into(),
                  journey_version: "v1".into(),
              })),
              profile_result: Mutex::new(Err(Status::not_found("n/a"))),
          };
          let adapter = RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
          let snap = adapter.resolve_snapshot(ResolveSnapshotReq {
              org_id: "o1".into(), journey_version: "v1".into(),
          }).await.expect("ok");
          assert_eq!(snap.snapshot_id, "snap-grpc");
      }

      #[tokio::test]
      async fn not_found_maps_to_permanent() {
          let mock = MockRegistry {
              snap_result: Mutex::new(Err(Status::not_found("unknown"))),
              profile_result: Mutex::new(Err(Status::not_found("n/a"))),
          };
          let adapter = RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
          let err = adapter.resolve_snapshot(ResolveSnapshotReq {
              org_id: "o1".into(), journey_version: "vX".into(),
          }).await.expect_err("fail");
          assert!(matches!(err, RegistryError::Permanent(_)));
      }

      #[tokio::test]
      async fn unavailable_maps_correctly() {
          let mock = MockRegistry {
              snap_result: Mutex::new(Err(Status::unavailable("down"))),
              profile_result: Mutex::new(Err(Status::not_found("n/a"))),
          };
          let adapter = RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
          let err = adapter.resolve_snapshot(ResolveSnapshotReq {
              org_id: "o1".into(), journey_version: "v1".into(),
          }).await.expect_err("fail");
          // After exhausting 5 retries on Unavailable, returns Unavailable or Transient
          assert!(matches!(err, RegistryError::Unavailable | RegistryError::Transient(_)));
      }

      #[tokio::test]
      async fn get_voice_profile_success() {
          let profile_id = uuid::Uuid::new_v4();
          let mock = MockRegistry {
              snap_result: Mutex::new(Err(Status::not_found("n/a"))),
              profile_result: Mutex::new(Ok(VoiceProfileProto {
                  id: profile_id.to_string(),
                  name: "grpc-bot".into(),
              })),
          };
          let adapter = RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
          let vp = adapter.get_voice_profile(&VoiceProfileId::from(profile_id))
              .await.expect("ok");
          assert_eq!(vp.name, "grpc-bot");
      }
  }
  ```

- [ ] **Step 7.2: Add `pub mod registry_grpc` to `src/lib.rs`**

  ```rust
  pub mod metrics;
  pub mod policies;
  pub mod registry_grpc;
  pub use registry_grpc::RegistryGrpcAdapter;

  #[cfg(feature = "registry-stub")]
  pub mod registry_stub;
  #[cfg(feature = "registry-stub")]
  pub use registry_stub::RegistryStubAdapter;
  ```

- [ ] **Step 7.3: Run tests**

  ```bash
  cargo test -p engagement-hub-adapters registry_grpc
  ```

  Expected: 4 tests pass.

- [ ] **Step 7.4: Commit**

  ```bash
  git add crates/engagement-hub-adapters/src/registry_grpc.rs crates/engagement-hub-adapters/src/lib.rs
  git commit -m "feat(adapters): add RegistryGrpcAdapter with tonic client + status mapping"
  ```

---

### Task 8: `PostCallHttpAdapter`

**Files:**

- Create: `crates/engagement-hub-adapters/src/post_call_http.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

**Endpoint map** (from `admin/backendv2/cmd/server/admin/calllog/services.go`):

- `GET /calls/{id}/transcription` → `{ "data": [{"message","role","audio_url","emotion",...}], "totalSize": int }`
- `GET /calls/{id}/summary` → `{ "data": {"summary","resolution","resolution_explanation"} }`
- `GET /calls/{id}/sentiment` → `{ "data": {"sentiment","justification"} }`
- `GET /calls/{id}/state` (output extraction) → `{ "data": [{"key","value","audio_file_paths"}] }`
- `GET /calls/{agent_id}/history-call?skip=&limit=&start_date=&end_date=&identity=&id=&batch_id=` → `{ "data": [...], "total_size": int }`
- `GET /calls/organizations/{org_id}?skip=&limit=&start_date=&end_date=&contact_number=&call_id=` → `{ "data": [...], "total_size": int }`

HTTP status mapping: 404/400/422 → `Permanent`; 503 → `Unavailable`; everything else → `Transient`.

- [ ] **Step 8.1: Write `post_call_http.rs`**

  ```rust
  use std::sync::Arc;

  use async_trait::async_trait;
  use engagement_hub_ports::{
      error::PostCallError,
      ports::PostCallPort,
      types::{
          CallLog, EngagementId, ListAgentCallLogsReq, ListOrgCallLogsReq, OutputExtraction,
          OutputField, Page, Sentiment, Summary, Transcript, TranscriptMessage,
      },
  };
  use reqwest::{Client, StatusCode};
  use serde::Deserialize;

  use crate::{
      metrics::AdapterMetrics,
      policies::{DEFAULT_RETRY, with_retry},
  };

  // ---------------------------------------------------------------------------
  // Private downstream response shapes
  // ---------------------------------------------------------------------------

  #[derive(Deserialize)]
  struct TranscriptionItem {
      message: String,
      role: String,
      audio_url: Option<String>,
      emotion: Option<String>,
  }

  #[derive(Deserialize)]
  #[serde(rename_all = "camelCase")]
  struct TranscriptionsResp {
      data: Vec<TranscriptionItem>,
      total_size: Option<i32>,
  }

  #[derive(Deserialize)]
  struct SummaryData {
      summary: String,
      resolution: Option<String>,
      resolution_explanation: Option<String>,
  }

  #[derive(Deserialize)]
  struct SummaryResp { data: Option<SummaryData> }

  #[derive(Deserialize)]
  struct SentimentData { sentiment: String, justification: String }

  #[derive(Deserialize)]
  struct SentimentResp { data: Option<SentimentData> }

  #[derive(Deserialize)]
  struct OutputExtractionItem { key: String, value: String }

  #[derive(Deserialize)]
  struct OutputExtractionResp { data: Vec<OutputExtractionItem> }

  #[derive(Deserialize)]
  struct CallLogItem {
      id: String,
      room_name: Option<String>,
      batch_id: Option<String>,
      duration: Option<i32>,
      identity: Option<String>,
      created_at: String,
  }

  #[derive(Deserialize)]
  struct CallLogListResp {
      data: Option<Vec<CallLogItem>>,
      total_size: Option<u32>,
  }

  // ---------------------------------------------------------------------------
  // HTTP helper
  // ---------------------------------------------------------------------------

  fn map_http_status(status: StatusCode, body: &str) -> PostCallError {
      match status {
          StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
              => PostCallError::Permanent(format!("{status}: {body}")),
          StatusCode::SERVICE_UNAVAILABLE
              => PostCallError::Unavailable,
          _ => PostCallError::Transient(format!("{status}: {body}")),
      }
  }

  async fn get_json<T: for<'de> Deserialize<'de>>(
      client: &Client,
      url: &str,
  ) -> Result<T, PostCallError> {
      let resp = client.get(url).send().await
          .map_err(|e| PostCallError::Transient(e.to_string()))?;
      if resp.status().is_success() {
          resp.json::<T>().await.map_err(|e| PostCallError::Permanent(e.to_string()))
      } else {
          let status = resp.status();
          let body = resp.text().await.unwrap_or_default();
          Err(map_http_status(status, &body))
      }
  }

  // ---------------------------------------------------------------------------
  // Adapter
  // ---------------------------------------------------------------------------

  pub struct PostCallHttpAdapter {
      client: Client,
      base_url: String,
      metrics: Arc<AdapterMetrics>,
  }

  impl PostCallHttpAdapter {
      pub fn new(client: Client, base_url: String, metrics: Arc<AdapterMetrics>) -> Self {
          Self { client, base_url, metrics }
      }
  }

  #[async_trait]
  impl PostCallPort for PostCallHttpAdapter {
      async fn get_transcript(&self, eng: &EngagementId) -> Result<Transcript, PostCallError> {
          let url = format!("{}/calls/{}/transcription", self.base_url, eng);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: TranscriptionsResp = get_json(&c, &u).await?;
                  Ok(Transcript {
                      messages: r.data.into_iter().map(|i| TranscriptMessage {
                          message: i.message,
                          role: i.role,
                          audio_url: i.audio_url,
                          emotion: i.emotion,
                      }).collect(),
                      total_size: r.total_size.unwrap_or(0),
                  })
              }
          }).await
      }

      async fn get_summary(&self, eng: &EngagementId) -> Result<Summary, PostCallError> {
          let url = format!("{}/calls/{}/summary", self.base_url, eng);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: SummaryResp = get_json(&c, &u).await?;
                  let d = r.data.ok_or_else(|| PostCallError::Permanent("empty summary data".into()))?;
                  Ok(Summary {
                      summary: d.summary,
                      resolution: d.resolution,
                      resolution_explanation: d.resolution_explanation,
                  })
              }
          }).await
      }

      async fn get_sentiment(&self, eng: &EngagementId) -> Result<Sentiment, PostCallError> {
          let url = format!("{}/calls/{}/sentiment", self.base_url, eng);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: SentimentResp = get_json(&c, &u).await?;
                  let d = r.data.ok_or_else(|| PostCallError::Permanent("empty sentiment data".into()))?;
                  Ok(Sentiment { label: d.sentiment, justification: d.justification })
              }
          }).await
      }

      async fn get_output_extraction(
          &self,
          eng: &EngagementId,
      ) -> Result<OutputExtraction, PostCallError> {
          let url = format!("{}/calls/{}/state", self.base_url, eng);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: OutputExtractionResp = get_json(&c, &u).await?;
                  Ok(OutputExtraction {
                      fields: r.data.into_iter()
                          .map(|f| OutputField { key: f.key, value: f.value })
                          .collect(),
                  })
              }
          }).await
      }

      async fn list_agent_call_logs(
          &self,
          req: ListAgentCallLogsReq,
      ) -> Result<Page<CallLog>, PostCallError> {
          let mut params = vec![];
          if let Some(v) = req.skip    { params.push(format!("skip={v}")) }
          if let Some(v) = req.limit   { params.push(format!("limit={v}")) }
          if let Some(v) = &req.start_date { params.push(format!("start_date={v}")) }
          if let Some(v) = &req.end_date   { params.push(format!("end_date={v}")) }
          if let Some(v) = &req.identity   { params.push(format!("identity={v}")) }
          if let Some(v) = &req.id         { params.push(format!("id={v}")) }
          if let Some(v) = &req.batch_id   { params.push(format!("batch_id={v}")) }
          let qs = if params.is_empty() { String::new() } else { format!("?{}", params.join("&")) };
          let url = format!("{}/calls/{}/history-call{}", self.base_url, req.agent_id, qs);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: CallLogListResp = get_json(&c, &u).await?;
                  Ok(Page {
                      items: r.data.unwrap_or_default().into_iter().map(|l| CallLog {
                          id: l.id, room_name: l.room_name, batch_id: l.batch_id,
                          duration: l.duration, identity: l.identity, created_at: l.created_at,
                      }).collect(),
                      total_size: r.total_size,
                      next_page_token: None,
                  })
              }
          }).await
      }

      async fn list_org_call_logs(
          &self,
          req: ListOrgCallLogsReq,
      ) -> Result<Page<CallLog>, PostCallError> {
          let mut params = vec![];
          if let Some(v) = req.skip           { params.push(format!("skip={v}")) }
          if let Some(v) = req.limit          { params.push(format!("limit={v}")) }
          if let Some(v) = &req.start_date    { params.push(format!("start_date={v}")) }
          if let Some(v) = &req.end_date      { params.push(format!("end_date={v}")) }
          if let Some(v) = &req.contact_number { params.push(format!("contact_number={v}")) }
          if let Some(v) = &req.call_id       { params.push(format!("call_id={v}")) }
          let qs = if params.is_empty() { String::new() } else { format!("?{}", params.join("&")) };
          let url = format!("{}/calls/organizations/{}{}", self.base_url, req.org_id, qs);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: CallLogListResp = get_json(&c, &u).await?;
                  Ok(Page {
                      items: r.data.unwrap_or_default().into_iter().map(|l| CallLog {
                          id: l.id, room_name: l.room_name, batch_id: l.batch_id,
                          duration: l.duration, identity: l.identity, created_at: l.created_at,
                      }).collect(),
                      total_size: r.total_size,
                      next_page_token: None,
                  })
              }
          }).await
      }
  }

  // ---------------------------------------------------------------------------
  // Tests — wiremock HTTP mocks
  // ---------------------------------------------------------------------------

  #[cfg(test)]
  mod tests {
      use super::*;
      use engagement_hub_ports::types::EngagementId;
      use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path, path_regex}};

      async fn make_adapter() -> (MockServer, PostCallHttpAdapter) {
          let server = MockServer::start().await;
          let adapter = PostCallHttpAdapter::new(
              Client::new(), server.uri(), AdapterMetrics::for_test(),
          );
          (server, adapter)
      }

      #[tokio::test]
      async fn get_transcript_maps_messages() {
          let (server, adapter) = make_adapter().await;
          let eng = EngagementId::new();
          Mock::given(method("GET")).and(path(format!("/calls/{eng}/transcription")))
              .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                  "data": [{"message":"hello","role":"agent","audio_url":null,"emotion":null}],
                  "totalSize": 1
              }))).mount(&server).await;
          let t = adapter.get_transcript(&eng).await.expect("ok");
          assert_eq!(t.messages.len(), 1);
          assert_eq!(t.messages[0].message, "hello");
          assert_eq!(t.total_size, 1);
      }

      #[tokio::test]
      async fn get_summary_unwraps_data() {
          let (server, adapter) = make_adapter().await;
          let eng = EngagementId::new();
          Mock::given(method("GET")).and(path(format!("/calls/{eng}/summary")))
              .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                  "data": {"summary":"great call","resolution":null,"resolution_explanation":null}
              }))).mount(&server).await;
          let s = adapter.get_summary(&eng).await.expect("ok");
          assert_eq!(s.summary, "great call");
      }

      #[tokio::test]
      async fn get_sentiment_unwraps_data() {
          let (server, adapter) = make_adapter().await;
          let eng = EngagementId::new();
          Mock::given(method("GET")).and(path(format!("/calls/{eng}/sentiment")))
              .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                  "data": {"sentiment":"positive","justification":"customer was happy"}
              }))).mount(&server).await;
          let s = adapter.get_sentiment(&eng).await.expect("ok");
          assert_eq!(s.label, "positive");
      }

      #[tokio::test]
      async fn get_output_extraction_maps_fields() {
          let (server, adapter) = make_adapter().await;
          let eng = EngagementId::new();
          Mock::given(method("GET")).and(path(format!("/calls/{eng}/state")))
              .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                  "data": [{"key":"name","value":"Alice","audio_file_paths":""}]
              }))).mount(&server).await;
          let oe = adapter.get_output_extraction(&eng).await.expect("ok");
          assert_eq!(oe.fields.len(), 1);
          assert_eq!(oe.fields[0].key, "name");
      }

      #[tokio::test]
      async fn list_agent_call_logs_returns_items() {
          let (server, adapter) = make_adapter().await;
          Mock::given(method("GET")).and(path_regex(r"/calls/agent-1/history-call"))
              .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                  "data": [{"id":"c1","created_at":"2026-01-01T00:00:00Z"}],
                  "total_size": 1
              }))).mount(&server).await;
          let page = adapter.list_agent_call_logs(ListAgentCallLogsReq {
              agent_id: "agent-1".into(), skip: None, limit: None, start_date: None,
              end_date: None, identity: None, id: None, batch_id: None,
          }).await.expect("ok");
          assert_eq!(page.items.len(), 1);
          assert_eq!(page.total_size, Some(1));
      }

      #[tokio::test]
      async fn list_org_call_logs_returns_items() {
          let (server, adapter) = make_adapter().await;
          Mock::given(method("GET")).and(path_regex(r"/calls/organizations/org-1"))
              .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                  "data": [{"id":"c2","created_at":"2026-01-02T00:00:00Z"}],
                  "total_size": 1
              }))).mount(&server).await;
          let page = adapter.list_org_call_logs(ListOrgCallLogsReq {
              org_id: "org-1".into(), skip: None, limit: None, start_date: None,
              end_date: None, contact_number: None, call_id: None,
          }).await.expect("ok");
          assert_eq!(page.items.len(), 1);
      }

      #[tokio::test]
      async fn not_found_maps_to_permanent() {
          let (server, adapter) = make_adapter().await;
          let eng = EngagementId::new();
          Mock::given(method("GET")).and(path(format!("/calls/{eng}/transcription")))
              .respond_with(ResponseTemplate::new(404)).mount(&server).await;
          let err = adapter.get_transcript(&eng).await.expect_err("fail");
          assert!(matches!(err, PostCallError::Permanent(_)));
      }

      #[tokio::test]
      async fn retries_on_503_then_succeeds() {
          let (server, adapter) = make_adapter().await;
          let eng = EngagementId::new();
          Mock::given(method("GET")).and(path(format!("/calls/{eng}/transcription")))
              .respond_with(ResponseTemplate::new(503)).up_to_n_times(1).mount(&server).await;
          Mock::given(method("GET")).and(path(format!("/calls/{eng}/transcription")))
              .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                  "data": [], "totalSize": 0
              }))).mount(&server).await;
          let t = adapter.get_transcript(&eng).await.expect("ok after retry");
          assert_eq!(t.messages.len(), 0);
      }
  }
  ```

- [ ] **Step 8.2: Add `pub mod post_call_http` to `src/lib.rs`**

  ```rust
  pub mod metrics;
  pub mod policies;
  pub mod post_call_http;
  pub mod registry_grpc;
  pub use post_call_http::PostCallHttpAdapter;
  pub use registry_grpc::RegistryGrpcAdapter;

  #[cfg(feature = "registry-stub")]
  pub mod registry_stub;
  #[cfg(feature = "registry-stub")]
  pub use registry_stub::RegistryStubAdapter;
  ```

- [ ] **Step 8.3: Run tests**

  ```bash
  cargo test -p engagement-hub-adapters post_call_http
  ```

  Expected: 8 tests pass.

- [ ] **Step 8.4: Commit**

  ```bash
  git add crates/engagement-hub-adapters/src/post_call_http.rs crates/engagement-hub-adapters/src/lib.rs
  git commit -m "feat(adapters): add PostCallHttpAdapter — 6 methods, real endpoints, wiremock tests"
  ```

---

### Task 9: `AnalyticsHttpAdapter`

**Files:**

- Create: `crates/engagement-hub-adapters/src/analytics_http.rs`
- Modify: `crates/engagement-hub-adapters/src/lib.rs`

**Endpoint map** (from `admin/backendv2/cmd/server/admin/analytic/service.go`):

- `GET /calls/agents/{agent_id}/analytics?metric=&granularity=&startDate=&endDate=` → `GetAnalyticsResponse` (direct, no `.data` wrapper)
- `GET /calls/agents/{agent_id}/metrics?…` → `{ "data": {"categories":[…], "series":[…]} }` (`.data` wrapper)
- `GET /calls/organizations/{org_id}/analytics?…` → same shape as agent analytics
- `GET /calls/organizations/{org_id}/metrics?…` → same `.data` wrapper as agent metrics

- [ ] **Step 9.1: Write `analytics_http.rs`**

  ```rust
  use std::sync::Arc;

  use async_trait::async_trait;
  use engagement_hub_ports::{
      error::AnalyticsError,
      ports::AnalyticsPort,
      types::{
          Analytics, GetAgentAnalyticsReq, GetAgentMetricsReq, GetOrgAnalyticsReq,
          GetOrgMetricsReq, Metrics,
      },
  };
  use reqwest::{Client, StatusCode};
  use serde::Deserialize;
  use std::collections::HashMap;

  use crate::{
      metrics::AdapterMetrics,
      policies::{DEFAULT_RETRY, with_retry},
  };

  // ---------------------------------------------------------------------------
  // Private downstream response shapes
  // ---------------------------------------------------------------------------

  #[derive(Deserialize)]
  struct AnalyticsResp {
      average_conversation_duration: f64,
      containment_rate: f64,
      customer_satisfaction_rate: f64,
      dropoff_rate: f64,
      escalation_rate: f64,
      total_inquiries: u32,
      #[serde(default)]
      category_counts: HashMap<String, u32>,
  }

  #[derive(Deserialize)]
  struct MetricData { categories: Vec<String>, series: Vec<f64> }

  #[derive(Deserialize)]
  struct MetricsResp { data: MetricData }

  // ---------------------------------------------------------------------------
  // HTTP helper
  // ---------------------------------------------------------------------------

  fn map_http_status(status: StatusCode, body: &str) -> AnalyticsError {
      match status {
          StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
              => AnalyticsError::Permanent(format!("{status}: {body}")),
          StatusCode::SERVICE_UNAVAILABLE => AnalyticsError::Unavailable,
          _ => AnalyticsError::Transient(format!("{status}: {body}")),
      }
  }

  async fn get_json<T: for<'de> Deserialize<'de>>(
      client: &Client,
      url: &str,
  ) -> Result<T, AnalyticsError> {
      let resp = client.get(url).send().await
          .map_err(|e| AnalyticsError::Transient(e.to_string()))?;
      if resp.status().is_success() {
          resp.json::<T>().await.map_err(|e| AnalyticsError::Permanent(e.to_string()))
      } else {
          let status = resp.status();
          let body = resp.text().await.unwrap_or_default();
          Err(map_http_status(status, &body))
      }
  }

  fn build_analytics_qs(metric: &Option<String>, granularity: &Option<String>, start: &Option<String>, end: &Option<String>) -> String {
      let mut params = vec![];
      if let Some(v) = metric     { params.push(format!("metric={v}")) }
      if let Some(v) = granularity { params.push(format!("granularity={v}")) }
      if let Some(v) = start      { params.push(format!("startDate={v}")) }
      if let Some(v) = end        { params.push(format!("endDate={v}")) }
      if params.is_empty() { String::new() } else { format!("?{}", params.join("&")) }
  }

  // ---------------------------------------------------------------------------
  // Adapter
  // ---------------------------------------------------------------------------

  pub struct AnalyticsHttpAdapter {
      client: Client,
      base_url: String,
      metrics: Arc<AdapterMetrics>,
  }

  impl AnalyticsHttpAdapter {
      pub fn new(client: Client, base_url: String, metrics: Arc<AdapterMetrics>) -> Self {
          Self { client, base_url, metrics }
      }
  }

  #[async_trait]
  impl AnalyticsPort for AnalyticsHttpAdapter {
      async fn get_agent_analytics(
          &self,
          req: GetAgentAnalyticsReq,
      ) -> Result<Analytics, AnalyticsError> {
          let qs = build_analytics_qs(&req.metric, &req.granularity, &req.start_date, &req.end_date);
          let url = format!("{}/calls/agents/{}/analytics{}", self.base_url, req.agent_id, qs);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "analytics", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: AnalyticsResp = get_json(&c, &u).await?;
                  Ok(Analytics {
                      average_conversation_duration: r.average_conversation_duration,
                      containment_rate: r.containment_rate,
                      customer_satisfaction_rate: r.customer_satisfaction_rate,
                      dropoff_rate: r.dropoff_rate,
                      escalation_rate: r.escalation_rate,
                      total_inquiries: r.total_inquiries,
                      category_counts: r.category_counts,
                  })
              }
          }).await
      }

      async fn get_agent_metrics(
          &self,
          req: GetAgentMetricsReq,
      ) -> Result<Metrics, AnalyticsError> {
          let qs = build_analytics_qs(&req.metric, &req.granularity, &req.start_date, &req.end_date);
          let url = format!("{}/calls/agents/{}/metrics{}", self.base_url, req.agent_id, qs);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "analytics", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: MetricsResp = get_json(&c, &u).await?;
                  Ok(Metrics { categories: r.data.categories, series: r.data.series })
              }
          }).await
      }

      async fn get_org_analytics(
          &self,
          req: GetOrgAnalyticsReq,
      ) -> Result<Analytics, AnalyticsError> {
          let qs = build_analytics_qs(&req.metric, &req.granularity, &req.start_date, &req.end_date);
          let url = format!("{}/calls/organizations/{}/analytics{}", self.base_url, req.org_id, qs);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "analytics", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: AnalyticsResp = get_json(&c, &u).await?;
                  Ok(Analytics {
                      average_conversation_duration: r.average_conversation_duration,
                      containment_rate: r.containment_rate,
                      customer_satisfaction_rate: r.customer_satisfaction_rate,
                      dropoff_rate: r.dropoff_rate,
                      escalation_rate: r.escalation_rate,
                      total_inquiries: r.total_inquiries,
                      category_counts: r.category_counts,
                  })
              }
          }).await
      }

      async fn get_org_metrics(
          &self,
          req: GetOrgMetricsReq,
      ) -> Result<Metrics, AnalyticsError> {
          let qs = build_analytics_qs(&req.metric, &req.granularity, &req.start_date, &req.end_date);
          let url = format!("{}/calls/organizations/{}/metrics{}", self.base_url, req.org_id, qs);
          let client = self.client.clone();
          let m = self.metrics.clone();
          with_retry(DEFAULT_RETRY, "analytics", Some(&m), move || {
              let c = client.clone(); let u = url.clone();
              async move {
                  let r: MetricsResp = get_json(&c, &u).await?;
                  Ok(Metrics { categories: r.data.categories, series: r.data.series })
              }
          }).await
      }
  }

  // ---------------------------------------------------------------------------
  // Tests — wiremock
  // ---------------------------------------------------------------------------

  #[cfg(test)]
  mod tests {
      use super::*;
      use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path_regex}};

      async fn make_adapter() -> (MockServer, AnalyticsHttpAdapter) {
          let server = MockServer::start().await;
          let adapter = AnalyticsHttpAdapter::new(
              Client::new(), server.uri(), AdapterMetrics::for_test(),
          );
          (server, adapter)
      }

      fn analytics_json() -> serde_json::Value {
          serde_json::json!({
              "average_conversation_duration": 45.5,
              "containment_rate": 0.75,
              "customer_satisfaction_rate": 0.85,
              "dropoff_rate": 0.1,
              "escalation_rate": 0.05,
              "total_inquiries": 100,
              "category_counts": {}
          })
      }

      fn metrics_json() -> serde_json::Value {
          serde_json::json!({"data": {"categories": ["mon","tue"], "series": [1.0, 2.0]}})
      }

      #[tokio::test]
      async fn get_agent_analytics_success() {
          let (server, adapter) = make_adapter().await;
          Mock::given(method("GET")).and(path_regex(r"/calls/agents/a1/analytics"))
              .respond_with(ResponseTemplate::new(200).set_body_json(analytics_json()))
              .mount(&server).await;
          let a = adapter.get_agent_analytics(GetAgentAnalyticsReq {
              agent_id: "a1".into(), metric: None, granularity: None,
              start_date: None, end_date: None,
          }).await.expect("ok");
          assert_eq!(a.total_inquiries, 100);
      }

      #[tokio::test]
      async fn get_agent_metrics_unwraps_data() {
          let (server, adapter) = make_adapter().await;
          Mock::given(method("GET")).and(path_regex(r"/calls/agents/a1/metrics"))
              .respond_with(ResponseTemplate::new(200).set_body_json(metrics_json()))
              .mount(&server).await;
          let m = adapter.get_agent_metrics(GetAgentMetricsReq {
              agent_id: "a1".into(), metric: None, granularity: None,
              start_date: None, end_date: None,
          }).await.expect("ok");
          assert_eq!(m.categories, vec!["mon", "tue"]);
          assert_eq!(m.series.len(), 2);
      }

      #[tokio::test]
      async fn get_org_analytics_success() {
          let (server, adapter) = make_adapter().await;
          Mock::given(method("GET")).and(path_regex(r"/calls/organizations/org1/analytics"))
              .respond_with(ResponseTemplate::new(200).set_body_json(analytics_json()))
              .mount(&server).await;
          let a = adapter.get_org_analytics(GetOrgAnalyticsReq {
              org_id: "org1".into(), metric: None, granularity: None,
              start_date: None, end_date: None,
          }).await.expect("ok");
          assert!((a.containment_rate - 0.75).abs() < 1e-9);
      }

      #[tokio::test]
      async fn get_org_metrics_success() {
          let (server, adapter) = make_adapter().await;
          Mock::given(method("GET")).and(path_regex(r"/calls/organizations/org1/metrics"))
              .respond_with(ResponseTemplate::new(200).set_body_json(metrics_json()))
              .mount(&server).await;
          let m = adapter.get_org_metrics(GetOrgMetricsReq {
              org_id: "org1".into(), metric: None, granularity: None,
              start_date: None, end_date: None,
          }).await.expect("ok");
          assert_eq!(m.series, vec![1.0, 2.0]);
      }

      #[tokio::test]
      async fn not_found_maps_to_permanent() {
          let (server, adapter) = make_adapter().await;
          Mock::given(method("GET")).and(path_regex(r"/calls/agents/missing/analytics"))
              .respond_with(ResponseTemplate::new(404)).mount(&server).await;
          let err = adapter.get_agent_analytics(GetAgentAnalyticsReq {
              agent_id: "missing".into(), metric: None, granularity: None,
              start_date: None, end_date: None,
          }).await.expect_err("fail");
          assert!(matches!(err, AnalyticsError::Permanent(_)));
      }
  }
  ```

- [ ] **Step 9.2: Finalize `src/lib.rs`**

  ```rust
  pub mod analytics_http;
  pub mod metrics;
  pub mod policies;
  pub mod post_call_http;
  pub mod registry_grpc;

  pub use analytics_http::AnalyticsHttpAdapter;
  pub use post_call_http::PostCallHttpAdapter;
  pub use registry_grpc::RegistryGrpcAdapter;

  #[cfg(feature = "registry-stub")]
  pub mod registry_stub;
  #[cfg(feature = "registry-stub")]
  pub use registry_stub::RegistryStubAdapter;
  ```

- [ ] **Step 9.3: Run tests**

  ```bash
  cargo test -p engagement-hub-adapters analytics_http
  ```

  Expected: 5 tests pass.

- [ ] **Step 9.4: Commit**

  ```bash
  git add crates/engagement-hub-adapters/src/analytics_http.rs crates/engagement-hub-adapters/src/lib.rs
  git commit -m "feat(adapters): add AnalyticsHttpAdapter — 4 methods, real endpoints, wiremock tests"
  ```

---

### Task 10: Replace old scaffold tests + full suite green

**Files:**

- Modify: `crates/engagement-hub-adapters/tests/adapter_scaffolding.rs`

- [ ] **Step 10.1: Replace the old panicking stubs with constructor smoke tests**

  ```rust
  // Verifies adapter structs can be constructed without panicking.

  use reqwest::Client;
  use engagement_hub_adapters::{
      AnalyticsHttpAdapter, PostCallHttpAdapter, RegistryGrpcAdapter,
      metrics::AdapterMetrics,
  };

  #[test]
  fn post_call_http_adapter_constructs() {
      let _ = PostCallHttpAdapter::new(
          Client::new(), "http://localhost:9999".into(), AdapterMetrics::for_test(),
      );
  }

  #[test]
  fn analytics_http_adapter_constructs() {
      let _ = AnalyticsHttpAdapter::new(
          Client::new(), "http://localhost:9999".into(), AdapterMetrics::for_test(),
      );
  }

  #[cfg(feature = "registry-stub")]
  #[test]
  fn registry_stub_adapter_constructs() {
      use engagement_hub_adapters::RegistryStubAdapter;
      let _ = RegistryStubAdapter::with_default_fixtures();
  }
  ```

  Note: `RegistryGrpcAdapter::new` requires a `Channel` — construction is tested in `registry_grpc.rs` inline tests via the mock server.

- [ ] **Step 10.2: Run the full adapter suite**

  ```bash
  cargo test -p engagement-hub-adapters --features registry-stub
  ```

  Expected: all tests across `metrics`, `policies`, `registry_stub`, `registry_grpc`, `post_call_http`, `analytics_http`, and `adapter_scaffolding` pass.

- [ ] **Step 10.3: Run the full workspace**

  ```bash
  cargo test --workspace
  ```

  Expected: all crates pass with no regressions.

- [ ] **Step 10.4: Configure panic-safety CI linter**

  Add the following step to `.github/workflows/ci.yml` (or equivalent CI config) under a code-quality job, after the test step:

  ```yaml
  - name: Lint adapter panic safety
    run: |
      if grep -rn '\.unwrap()\|\.expect(' crates/engagement-hub-adapters/src/ \
           --include='*.rs' \
           | grep -v 'policies\.rs' \
           | grep -v '#\[cfg(test)\]' \
           | grep -qv '^$'; then
        echo "ERROR: .unwrap() or .expect() found in adapter src outside policies.rs"
        grep -rn '\.unwrap()\|\.expect(' crates/engagement-hub-adapters/src/ \
             --include='*.rs' | grep -v 'policies\.rs'
        exit 1
      fi
  ```

  This fails the CI if `.unwrap()` or `.expect()` appear outside `policies.rs` in adapter source files.

  Check whether a CI workflow file exists first:

  ```bash
  ls .github/workflows/ 2>/dev/null || echo "no CI yet"
  ```

  If no CI file exists, create a minimal `.github/workflows/ci.yml` with just this check plus the existing `cargo test --workspace` step. If a CI file exists, add this step to the appropriate job.

- [ ] **Step 10.5: Commit**

  ```bash
  git add crates/engagement-hub-adapters/tests/adapter_scaffolding.rs
  # Also add CI file if created/modified
  git add .github/ 2>/dev/null || true
  git commit -m "feat(adapters): replace scaffold stubs with real smoke tests + add panic-safety CI lint"
  ```

---

### Deferred

- **Write adapters** (JourneyManagerGrpcAdapter, VoiceManagerHttpAdapter) — T1-04
- **Wire adapters in `main.rs`** with `EH_REGISTRY_GRPC_ENDPOINT` config — T1-06 (orchestrator)
- **`AdapterMetrics` registration with main `Metrics` struct** — T1-06
- **Deadline propagation from caller to adapter** (Option B from brainstorm) — T1-06
- **Registry `get_voice_profile` retry count** — currently DEFAULT_RETRY (3 attempts); update to match final Registry SLA when service ships
- **buf.gen.yaml Rust plugin** — not needed; tonic-build in build.rs is sufficient for internal proto
