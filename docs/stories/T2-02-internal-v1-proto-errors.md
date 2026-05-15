# T2-02: Internal v1 proto + errors.proto + status mapping

## Context

Defines two pieces of the proto surface that the external v1 package depends on but keeps separate: (1) the typed `EngagementError` message + all 14 error codes with their gRPC status mappings, serialized into `google.rpc.Status.details` — consumed by every SDK call site and by typed-error handling in T2-05; (2) the `revocall.engagement.internal.v1` package with `EngagementHubInternal.NotifyEngagementLifecycleEvent`, the intra-cluster surface that ai-handler (T4) uses to report lifecycle events back to EH core. Splitting internal from external prevents accidental exposure of cluster-only RPCs to public consumers.

## Story details

- **Track:** T2 — Proto + Go SDK
- **Owner:** EH team
- **PRD refs:** §5.4 (errors), §5.7 (status mapping), §6 (internal)
- **Depends on:** T2-01

## Acceptance criteria

- `revocall.engagement.v1.errors.proto`: EngagementError message; all 14 error codes with wire-compat numeric values preserved from engagement.proto; per-enum-value gRPC status mapping comments on every code, plus a header comment pointing at PRD §5 as canonical authority. Full mapping: ROUTE_RESOLUTION_FAILED → FAILED_PRECONDITION, JOURNEY_VERSION_NOT_FOUND → FAILED_PRECONDITION, TELEPHONY_NOT_AVAILABLE → FAILED_PRECONDITION, VOICE_PROFILE_NOT_FOUND → FAILED_PRECONDITION, VOICE_SESSION_REJECTED → ABORTED, JOURNEY_EXECUTION_REJECTED → ABORTED, REGISTRY_UNAVAILABLE → UNAVAILABLE, CONTACT_UNREACHABLE → FAILED_PRECONDITION, CALL_ENDED_WITH_ERROR → FAILED_PRECONDITION, ORG_QUOTA_EXCEEDED → RESOURCE_EXHAUSTED, ENGAGEMENT_NOT_FOUND → NOT_FOUND, ENGAGEMENT_ALREADY_TERMINAL → FAILED_PRECONDITION, REQUEST_ID_CONFLICT → ALREADY_EXISTS, INTERNAL → INTERNAL
- EngagementError serialized into `google.rpc.Status.details` per Connect-Go convention (documented as file-level comment; no google/rpc proto import required)
- `errors.proto` declares `option go_package = "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/gen/go/revocall/engagement/v1;engagementv1"` (same as other v1 files)
- `revocall.engagement.internal.v1` package: `service.proto` with `EngagementHubInternal.NotifyEngagementLifecycleEvent`; request fields (organization_id, engagement_id, request_id, event, occurred_at); LifecycleEvent oneof with 7 placeholder empty-message types (DialingStarted, CallAnswered, VoicemailDetected, CallTransferred, CallEnded, JourneyCompleted, JourneyFailed); each placeholder message carries `reserved 1 to 99;` for future field additions
- `internal/v1/service.proto` declares `option go_package = "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/gen/go/revocall/engagement/internal/v1;engagementinternalv1"`
- `EngagementHubInternal` service uses `// buf:lint:ignore SERVICE_SUFFIX` (mirrors v1/service.proto's pattern for `EngagementHub`)
- Internal auth notes documented inline: plaintext intra-cluster, NetworkPolicy restricted, x-revolab-caller-service metadata, idempotency on (engagement_id, request_id)

## Definition of done

- Both packages committed
- Separate from external v1
- `buf lint` green (STANDARD ruleset; lint suppression for SERVICE_SUFFIX on EngagementHubInternal is expected and correct)
- `buf breaking` will produce FILE-level failures for the EngagementError/EngagementErrorCode move — intentional and acknowledged; no Go SDK consumers exist yet. PR description must document this explicitly.

## Design (approved 2026-05-15)

### Approach: Approach A — simple extract, single buf module

#### errors.proto extraction

Create `proto/revocall/engagement/v1/errors.proto` (package `revocall.engagement.v1`). Move `EngagementError` message and `EngagementErrorCode` enum out of `engagement.proto` into this file. Add gRPC status mapping as inline comments grouped by status code (per the table in PRD §5.4). Add a file-level comment noting that `EngagementError` is serialized into `google.rpc.Status.details` per Connect-Go convention.

`engagement.proto` drops both definitions and adds `import "revocall/engagement/v1/errors.proto"`. All field numbers and wire formats are preserved — only the file ownership changes. No other v1 files need touching.

Rationale: one source of truth for all error-related proto definitions. Splitting the mapping comments into a separate file while leaving the types in `engagement.proto` would mean maintaining two files for the same concern.

#### internal.v1 package

Create `proto/revocall/engagement/internal/v1/service.proto` (package `revocall.engagement.internal.v1`). Contains:

- `EngagementHubInternal` service with single RPC `NotifyEngagementLifecycleEvent`; requires `// buf:lint:ignore SERVICE_SUFFIX` (name doesn't end in `Service`, mirroring v1/service.proto pattern)
- `option go_package = "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/gen/go/revocall/engagement/internal/v1;engagementinternalv1"`
- `import "google/protobuf/timestamp.proto"` (for `occurred_at`)
- `NotifyEngagementLifecycleEventRequest` (organization_id, engagement_id, request_id, event, occurred_at)
- `NotifyEngagementLifecycleEventResponse` (empty)
- `LifecycleEvent` oneof with 7 placeholder message types: DialingStarted, CallAnswered, VoicemailDetected, CallTransferred, CallEnded, JourneyCompleted, JourneyFailed; each empty with `reserved 1 to 99;` for future field additions
- Auth notes as file-level comments (plaintext intra-cluster, NetworkPolicy restriction, x-revolab-caller-service metadata, idempotency on engagement_id+request_id, v2 mTLS note)

Internal.v1 intentionally has **no import from `revocall.engagement.v1`** — it identifies engagements by string IDs only, preventing coupling back to the external surface.

**buf.yaml unchanged** — single module over `proto/`, STANDARD lint, FILE breaking. The internal/v1 path falls under the existing module automatically.

**buf breaking note** — moving `EngagementError`/`EngagementErrorCode` out of `engagement.proto` triggers a FILE-level breaking change against main. Intentional and acceptable: no Go SDK has been generated from T2-01, so there are zero consumers of those generated types. Document explicitly in PR description.

#### Two commits, one PR

1. `feat(proto): errors.proto — extract EngagementError + status mapping from engagement.proto`
2. `feat(proto): internal/v1 — EngagementHubInternal.NotifyEngagementLifecycleEvent`

### File layout

```
proto/
└── revocall/engagement/
    ├── v1/
    │   ├── engagement.proto     ← MODIFIED: remove EngagementError+EngagementErrorCode, add import
    │   ├── errors.proto         ← NEW: EngagementError + EngagementErrorCode + status mapping
    │   ├── control.proto        ← unchanged
    │   ├── query.proto          ← unchanged
    │   ├── watch.proto          ← unchanged
    │   ├── proxied.proto        ← unchanged
    │   └── service.proto        ← unchanged
    └── internal/v1/
        └── service.proto        ← NEW: EngagementHubInternal + LifecycleEvent placeholder types
buf.yaml                         ← unchanged
```

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `EngagementError`/`EngagementErrorCode` into a dedicated `errors.proto` and add the `revocall.engagement.internal.v1` package with the `EngagementHubInternal.NotifyEngagementLifecycleEvent` RPC.

**Architecture:** Two independent proto changes in one PR, each in its own commit. `errors.proto` stays in the same `revocall.engagement.v1` package — `engagement.proto` just imports from it. `internal/v1/service.proto` is a new package under the same buf module, intentionally decoupled (no imports from external v1).

**Tech Stack:** Protocol Buffers, buf 1.69.0 (`buf lint`, `buf build`). No Rust/Go code changes in this story.

**Worktree:** `/Users/chunzhe/Projects/ReVoCall-Engagement.t2-02` on `feat/21-internal-proto`.

---

### Task 1: Create `errors.proto` — extract error types from `engagement.proto`

**Files:**

- Create: `proto/revocall/engagement/v1/errors.proto`
- Modify: `proto/revocall/engagement/v1/engagement.proto`

- [ ] **Step 1: Verify buf baseline is clean before touching anything**

  From the worktree root:

  ```bash
  cd /Users/chunzhe/Projects/ReVoCall-Engagement.t2-02
  buf lint
  buf build
  ```

  Expected: both commands exit 0 with no output.

- [ ] **Step 2: Create `proto/revocall/engagement/v1/errors.proto`**

  Create the file with this exact content:

  ```proto
  syntax = "proto3";

  package revocall.engagement.v1;

  option go_package = "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/gen/go/revocall/engagement/v1;engagementv1";

  // EngagementError is serialized into google.rpc.Status.details per the
  // Connect-Go / tonic convention. The gRPC status code is selected by the
  // server based on the EngagementErrorCode value; see PRD §5 for the
  // canonical mapping table.

  message EngagementError {
    EngagementErrorCode code               = 1;
    string message                         = 2;
    optional string downstream_service     = 3;
    map<string, string> details            = 4;
  }

  enum EngagementErrorCode {
    ENGAGEMENT_ERROR_CODE_UNSPECIFIED                 = 0;
    ENGAGEMENT_ERROR_CODE_ROUTE_RESOLUTION_FAILED     = 1;  // gRPC: FAILED_PRECONDITION
    ENGAGEMENT_ERROR_CODE_JOURNEY_VERSION_NOT_FOUND   = 2;  // gRPC: FAILED_PRECONDITION
    ENGAGEMENT_ERROR_CODE_TELEPHONY_NOT_AVAILABLE     = 3;  // gRPC: FAILED_PRECONDITION
    ENGAGEMENT_ERROR_CODE_VOICE_PROFILE_NOT_FOUND     = 4;  // gRPC: FAILED_PRECONDITION
    ENGAGEMENT_ERROR_CODE_VOICE_SESSION_REJECTED      = 10; // gRPC: ABORTED
    ENGAGEMENT_ERROR_CODE_JOURNEY_EXECUTION_REJECTED  = 11; // gRPC: ABORTED
    ENGAGEMENT_ERROR_CODE_REGISTRY_UNAVAILABLE        = 12; // gRPC: UNAVAILABLE
    ENGAGEMENT_ERROR_CODE_CONTACT_UNREACHABLE         = 20; // gRPC: FAILED_PRECONDITION
    ENGAGEMENT_ERROR_CODE_CALL_ENDED_WITH_ERROR       = 21; // gRPC: FAILED_PRECONDITION
    ENGAGEMENT_ERROR_CODE_ORG_QUOTA_EXCEEDED          = 30; // gRPC: RESOURCE_EXHAUSTED
    ENGAGEMENT_ERROR_CODE_ENGAGEMENT_NOT_FOUND        = 31; // gRPC: NOT_FOUND
    ENGAGEMENT_ERROR_CODE_ENGAGEMENT_ALREADY_TERMINAL = 32; // gRPC: FAILED_PRECONDITION
    ENGAGEMENT_ERROR_CODE_REQUEST_ID_CONFLICT         = 33; // gRPC: ALREADY_EXISTS
    ENGAGEMENT_ERROR_CODE_INTERNAL                    = 99; // gRPC: INTERNAL
    reserved 1000 to 1999;
  }
  ```

- [ ] **Step 3: Update `proto/revocall/engagement/v1/engagement.proto`**

  Replace the entire file with this content (removes the `// ── Error type ──` block and `EngagementErrorCode` enum, adds the import):

  ```proto
  syntax = "proto3";

  package revocall.engagement.v1;

  import "google/protobuf/duration.proto";
  import "google/protobuf/timestamp.proto";
  import "revocall/engagement/v1/errors.proto";

  option go_package = "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/gen/go/revocall/engagement/v1;engagementv1";

  // ── Core entity ───────────────────────────────────────────────────────────────

  message Engagement {
    string engagement_id              = 1;
    string organization_id            = 2;
    Channel channel                   = 3;
    EngagementMode mode               = 4;
    JourneyVersionRef journey_version = 5;
    optional string snapshot_id       = 6;
    ContactRef contact                = 7;
    optional string batch_id          = 8;
    PrincipalRef created_by           = 9;
    EngagementStatus status           = 10;
    optional EngagementError error    = 11;
    map<string, string> metadata      = 12;
    google.protobuf.Timestamp created_at          = 20;
    google.protobuf.Timestamp updated_at          = 21;
    optional google.protobuf.Timestamp started_at = 22;
    optional google.protobuf.Timestamp ended_at   = 23;
    reserved 100 to 199;
  }

  // ── Reference types ───────────────────────────────────────────────────────────

  message JourneyVersionRef {
    string journey_id = 1;
    string version    = 2;
  }

  message ContactRef {
    oneof identity {
      string contact_id = 1;
      PhoneNumber phone = 2;
    }
    optional string display_name = 3;
  }

  message PhoneNumber {
    string e164 = 1;
  }

  message PrincipalRef {
    oneof principal {
      string user_id         = 1;
      string service_account = 2;
    }
  }

  // VoiceConfig is used in control.proto request messages (Start, Resolve).
  // It is intentionally absent from the Engagement entity (per PRD §5).
  message VoiceConfig {
    optional string voice_profile_ref              = 1;
    optional string telephony_id                   = 2;
    optional google.protobuf.Duration max_duration = 3;
    optional bool record                           = 4;
    reserved 100 to 199;
  }

  // ── Enums ─────────────────────────────────────────────────────────────────────

  enum Channel {
    CHANNEL_UNSPECIFIED = 0;
    CHANNEL_VOICE       = 1;
    reserved 2 to 9;
    reserved 1000 to 1999;
  }

  enum EngagementMode {
    ENGAGEMENT_MODE_UNSPECIFIED = 0;
    ENGAGEMENT_MODE_OUTBOUND    = 1;
    reserved 2;
    reserved 1000 to 1999;
  }

  enum EngagementStatus {
    ENGAGEMENT_STATUS_UNSPECIFIED = 0;
    ENGAGEMENT_STATUS_PENDING     = 1;
    ENGAGEMENT_STATUS_INVOKING    = 2;
    ENGAGEMENT_STATUS_LIVE        = 3;
    ENGAGEMENT_STATUS_COMPLETED   = 4;
    ENGAGEMENT_STATUS_FAILED      = 5;
    ENGAGEMENT_STATUS_CANCELLED   = 6;
    reserved 100 to 199;
    reserved 1000 to 1999;
  }

  enum EngagementEventType {
    ENGAGEMENT_EVENT_TYPE_UNSPECIFIED          = 0;
    ENGAGEMENT_EVENT_TYPE_STREAM_OPENED        = 1;
    ENGAGEMENT_EVENT_TYPE_CREATED              = 2;
    ENGAGEMENT_EVENT_TYPE_ROUTE_RESOLVED       = 3;
    ENGAGEMENT_EVENT_TYPE_INVOCATION_REQUESTED = 4;
    ENGAGEMENT_EVENT_TYPE_JOURNEY_BOUND        = 5;
    ENGAGEMENT_EVENT_TYPE_VOICE_SESSION_BOUND  = 6;
    ENGAGEMENT_EVENT_TYPE_LIVE                 = 7;
    ENGAGEMENT_EVENT_TYPE_STATUS_UPDATE        = 8;
    ENGAGEMENT_EVENT_TYPE_COMPLETED            = 9;
    ENGAGEMENT_EVENT_TYPE_FAILED               = 10;
    ENGAGEMENT_EVENT_TYPE_CANCELLED            = 11;
    ENGAGEMENT_EVENT_TYPE_HEARTBEAT            = 12;
    ENGAGEMENT_EVENT_TYPE_STREAM_OVERFLOW      = 13;
    reserved 1000 to 1999;
  }
  ```

- [ ] **Step 4: Verify `buf lint` and `buf build` pass**

  ```bash
  buf lint
  buf build
  ```

  Expected: both exit 0 with no output. If lint complains about `IMPORT_USED` or `IMPORT_NO_PUBLIC`, verify the import is placed after the existing google imports. If build fails with "undefined EngagementError", the import line is missing or has a typo in the path.

- [ ] **Step 5: Commit**

  ```bash
  git add proto/revocall/engagement/v1/errors.proto \
          proto/revocall/engagement/v1/engagement.proto
  git commit -m "feat(proto): errors.proto — extract EngagementError + status mapping from engagement.proto"
  ```

---

### Task 2: Create `internal/v1/service.proto` — `EngagementHubInternal` package

**Files:**

- Create: `proto/revocall/engagement/internal/v1/service.proto`

- [ ] **Step 1: Create the directory and file**

  ```bash
  mkdir -p proto/revocall/engagement/internal/v1
  ```

  Create `proto/revocall/engagement/internal/v1/service.proto` with this exact content:

  ```proto
  syntax = "proto3";

  package revocall.engagement.internal.v1;

  import "google/protobuf/timestamp.proto";

  option go_package = "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/gen/go/revocall/engagement/internal/v1;engagementinternalv1";

  // Auth (v1): plaintext intra-cluster, no mTLS.
  // NetworkPolicy: ingress restricted to voice-manager + journey-manager namespaces.
  // Caller MUST set x-revolab-caller-service metadata (unsigned; audit-only).
  // Idempotency: (engagement_id, request_id) pair; duplicate events are discarded.
  // Auth (v2): mTLS with SPIFFE SAN when Platform Service Auth PRD ships.

  // buf:lint:ignore SERVICE_SUFFIX
  service EngagementHubInternal {
    rpc NotifyEngagementLifecycleEvent(NotifyEngagementLifecycleEventRequest)
        returns (NotifyEngagementLifecycleEventResponse);
  }

  message NotifyEngagementLifecycleEventRequest {
    string organization_id                = 1;
    string engagement_id                  = 2;
    string request_id                     = 3;
    LifecycleEvent event                  = 4;
    google.protobuf.Timestamp occurred_at = 5;
  }

  message NotifyEngagementLifecycleEventResponse {}

  message LifecycleEvent {
    oneof event {
      DialingStarted    dialing_started    = 1;
      CallAnswered      call_answered      = 2;
      VoicemailDetected voicemail_detected = 3;
      CallTransferred   call_transferred   = 4;
      CallEnded         call_ended         = 5;
      JourneyCompleted  journey_completed  = 6;
      JourneyFailed     journey_failed     = 7;
    }
  }

  // Placeholder event types — payloads defined when ai-handler (T4) is implemented.
  // reserved 1 to 99 holds field-number space for future payload fields without renumbering.
  message DialingStarted    { reserved 1 to 99; }
  message CallAnswered      { reserved 1 to 99; }
  message VoicemailDetected { reserved 1 to 99; }
  message CallTransferred   { reserved 1 to 99; }
  message CallEnded         { reserved 1 to 99; }
  message JourneyCompleted  { reserved 1 to 99; }
  message JourneyFailed     { reserved 1 to 99; }
  ```

  **Important:** Do NOT add any import from `revocall.engagement.v1`. The internal package identifies engagements by string IDs only — this isolation is intentional (see Design section above).

- [ ] **Step 2: Verify `buf lint` and `buf build` pass**

  ```bash
  buf lint
  buf build
  ```

  Expected: both exit 0 with no output. The `// buf:lint:ignore SERVICE_SUFFIX` comment silences the one expected lint warning.

  **Note on `buf breaking`:** If you run `buf breaking --against .git#branch=main`, it will report FILE-level breaking changes for the `EngagementError`/`EngagementErrorCode` move (Task 1). This is intentional — no Go SDK has been generated from T2-01, so there are zero consumers. Do not fix this; document it in the PR description.

- [ ] **Step 3: Commit**

  ```bash
  git add proto/revocall/engagement/internal/v1/service.proto
  git commit -m "feat(proto): internal/v1 — EngagementHubInternal.NotifyEngagementLifecycleEvent"
  ```

---

### Task 3: Open the PR

- [ ] **Step 1: Push the branch**

  ```bash
  git push -u origin feat/21-internal-proto
  ```

- [ ] **Step 2: Create the PR**

  ```bash
  gh pr create \
    --title "T2-02: errors.proto extraction + internal.v1 EngagementHubInternal" \
    --body "$(cat <<'EOF'
  ## Summary

  - Moves `EngagementError` and `EngagementErrorCode` out of `engagement.proto` into a new `errors.proto` (same `revocall.engagement.v1` package). `engagement.proto` imports from it — one source of truth for all error-related proto definitions.
  - Adds `revocall.engagement.internal.v1` package with `EngagementHubInternal.NotifyEngagementLifecycleEvent` and 7 placeholder lifecycle event types. Intentionally decoupled from external v1 (no cross-package imports).

  ## buf breaking note

  `buf breaking --against main` will report FILE-level failures for the `EngagementError`/`EngagementErrorCode` move from `engagement.proto` to `errors.proto`. This is intentional: wire format and field numbers are unchanged; no Go SDK has been generated from T2-01, so there are zero consumers of those generated types.

  ## Test plan

  - [x] `buf lint` — passes (STANDARD ruleset; `SERVICE_SUFFIX` suppressed on `EngagementHubInternal` per existing pattern)
  - [x] `buf build` — passes
  - [x] `buf breaking` — intentional FILE-level failures documented above
  EOF
  )"
  ```
