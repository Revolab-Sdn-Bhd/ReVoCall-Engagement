# T2-05: Typed Error type + sentinels + classification helpers

**Issue:** #24 | **Branch:** feat/24-typed-error | **Date:** 2026-05-17

## Brainstorm

### Problem

The Engagement Hub wire contract serializes failures as a `revocall.engagement.v1.EngagementError` proto attached to `google.rpc.Status.details` (T2-02). Generated Go stubs (T2-03) surface these as `*connect.Error` instances whose details are opaque `*anypb.Any` blobs. Consumers — outbound dispatcher's retry loop, admin-backend's HTTP error mapping, ai-handler's lifecycle reporter — need to branch on specific failure modes (`is this a request_id conflict? a quota issue? a transient outage?`) without poking at gRPC status codes or unpacking proto details inline at every call site.

T2-05 introduces the SDK-side ergonomic layer: a typed `Error` with one sentinel per code, classification helpers (`IsTransient`, `IsTerminal`, `IsClientError`, `IsServerError`) for retry middleware, and a single conversion function from `*connect.Error`. Every SDK RPC wrapper (T2-06, T2-07) will funnel responses through this layer.

### Options considered

#### A. Root `engagementhub` package (chosen)

Put `errors.go` directly in `clients/go/engagementhub/` as `package engagementhub`. Consumers import `eh "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub"` and write `errors.Is(err, eh.ErrRequestIDConflict)` / `eh.IsTransient(err)`. Matches the convention used by mature Go SDKs (AWS SDK, Google Cloud SDK) — top-level error types live at the SDK root. No naming collisions.

#### B. `shared/eherrors/` sub-package

Follows the `shared/idempotency/` convention from T2-10. The `eh` prefix avoids the stdlib `errors` collision but reads awkwardly (`eherrors.ErrRequestIDConflict` is noisier than `eh.ErrRequestIDConflict`). The `shared/` directory is for utilities used across the SDK and external callers; the error type is the SDK's public surface, which belongs at the root.

#### C. `shared/errors/` sub-package

Same path as B but named `errors`. Every consumer file that imports both stdlib `errors` and this package would have to alias one of them (`import stderrors "errors"`). Poor ergonomics for the most common error-handling pattern.

### Decision

**Package:** Root `engagementhub` package — single file `clients/go/engagementhub/errors.go`. Re-export the gen enum as `type EngagementErrorCode = engagementv1.EngagementErrorCode` and define short-form constants (`CodeRouteResolutionFailed`, etc.) so consumers never reference `internal/gen/...` directly.

**`Error` struct:** `Code`, `Message`, `DownstreamService` (optional), `Details` (`map[string]string`), and an unexported `cause` surfaced via `Unwrap()`. `Error()` returns `"engagement_hub: <CODE>: <message>"` (plus `" (downstream=X)"` when set). `Is(target error) bool` matches by `Code` only — this is what lets `errors.Is(wrapped, ErrRequestIDConflict)` work without pointer-equality concerns.

**Sentinels:** 14 exported `*Error` values (one per non-UNSPECIFIED code), each constructed as `&Error{Code: Code…}`. Consumers compare via `errors.Is` only.

**Classifiers:**

- `IsTransient`: `CodeRegistryUnavailable`, `CodeInternal` — the retry middleware retries these
- `IsTerminal`: `CodeEngagementNotFound`, `CodeEngagementAlreadyTerminal`, `CodeContactUnreachable`, `CodeRequestIDConflict`, `CodeOrgQuotaExceeded` — never retry
- `IsClientError`: every code except `CodeRegistryUnavailable` and `CodeInternal` (i.e. everything that isn't a 5xx-style gRPC status)
- `IsServerError`: `CodeRegistryUnavailable`, `CodeInternal`

The three ABORTED / call-lifecycle codes (`CodeVoiceSessionRejected`, `CodeJourneyExecutionRejected`, `CodeCallEndedWithError`) are intentionally **neither** transient nor terminal — the dispatcher's retry loop treats "neither" as "don't auto-retry, let the caller decide based on context". gRPC convention permits retrying ABORTED, but middleware-level retries without business context are riskier than skipping.

Each classifier uses `errors.As(err, &e)` to unwrap through wrapping; non-`*Error` inputs return `false`.

**Conversion:** Single function `FromConnectError(err error) (*Error, bool)`. Uses `errors.As` to find a `*connect.Error`, iterates `connectErr.Details()`, and unmarshals the first detail whose type URL matches `revocall.engagement.v1.EngagementError`. Preserves the original `err` as `cause` for `Unwrap()`. Returns `(nil, false)` when no matching detail is present (caller keeps the original error).

### Out of scope

- RPC wrappers that *invoke* `FromConnectError` — that is T2-06/T2-07
- Server-side helper that builds a `*connect.Error` *from* an `*Error` — only client-side conversion is needed today
- A `Wrap(cause error, code) *Error` helper — `NewError(code, message, cause)` and direct struct construction inside the package cover known cases; add `Wrap` later if external call sites demand it

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the SDK-side typed `Error` for `revocall.engagement.v1.EngagementError`, with 14 sentinels, four classification helpers, and `FromConnectError` conversion.

**Architecture:** Single new file `clients/go/engagementhub/errors.go` plus its companion test file. Package `engagementhub` (root of the SDK module). Imports the generated proto types from the internal gen package and `connectrpc.com/connect`. No new module dependencies.

**Tech Stack:** Go 1.26, `connectrpc.com/connect` v1.19.2, stdlib `go test`.

### Design decisions

- Single file in root SDK package — error types are public surface; no `shared/` sub-package
- `EngagementErrorCode` re-exported as type alias plus short-form constants — consumers never import `internal/gen/...`
- `NewError(code, message, cause)` is the canonical constructor for non-sentinel errors with a cause; tests need it because `cause` is unexported
- Sentinels are `&Error{Code: …}` literals; `Error.Is(target)` matches by `Code` so `errors.Is` works through wrapping
- Classifiers use `errors.As` so they unwrap through `fmt.Errorf("wrap: %w", err)`
- `FromConnectError` extracts the first `EngagementError` detail via `ErrorDetail.Value()` and type-asserts to `*engagementv1.EngagementError` — no fragile type-URL string matching

### File map

| Action | Path | Responsibility |
| ------ | ---- | -------------- |
| Create | `clients/go/engagementhub/errors.go` | `Error` struct, `NewError`, sentinels, classifiers, `FromConnectError` |
| Create | `clients/go/engagementhub/errors_test.go` | All tests |

---

### Task 1: Write failing tests (red)

**Files:**

- Create: `clients/go/engagementhub/errors_test.go`

- [ ] **Step 1: Create the test file**

```go
package engagementhub_test

import (
 "errors"
 "fmt"
 "testing"

 "connectrpc.com/connect"
 eh "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub"
 engagementv1 "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen/revocall/engagement/v1"
)

func TestError_Format(t *testing.T) {
 e := &eh.Error{Code: eh.CodeRequestIDConflict, Message: "duplicate"}
 want := "engagement_hub: ENGAGEMENT_ERROR_CODE_REQUEST_ID_CONFLICT: duplicate"
 if got := e.Error(); got != want {
  t.Fatalf("Error() = %q, want %q", got, want)
 }

 e2 := &eh.Error{Code: eh.CodeRegistryUnavailable, Message: "down", DownstreamService: "registry"}
 want2 := "engagement_hub: ENGAGEMENT_ERROR_CODE_REGISTRY_UNAVAILABLE: down (downstream=registry)"
 if got := e2.Error(); got != want2 {
  t.Fatalf("Error() with downstream = %q, want %q", got, want2)
 }
}

func TestError_Unwrap(t *testing.T) {
 cause := errors.New("underlying")
 e := eh.NewError(eh.CodeInternal, "boom", cause)
 if !errors.Is(e, cause) {
  t.Fatal("errors.Is(e, cause) should be true via Unwrap")
 }
}

func TestErrorsIs_MatchesSentinelThroughWrap(t *testing.T) {
 e := &eh.Error{Code: eh.CodeRequestIDConflict, Message: "x"}
 wrapped := fmt.Errorf("rpc failed: %w", e)
 if !errors.Is(wrapped, eh.ErrRequestIDConflict) {
  t.Fatal("errors.Is should match wrapped sentinel by code")
 }
}

func TestSentinels_AllDistinct(t *testing.T) {
 sentinels := []*eh.Error{
  eh.ErrRouteResolutionFailed,
  eh.ErrJourneyVersionNotFound,
  eh.ErrTelephonyNotAvailable,
  eh.ErrVoiceProfileNotFound,
  eh.ErrVoiceSessionRejected,
  eh.ErrJourneyExecutionRejected,
  eh.ErrRegistryUnavailable,
  eh.ErrContactUnreachable,
  eh.ErrCallEndedWithError,
  eh.ErrOrgQuotaExceeded,
  eh.ErrEngagementNotFound,
  eh.ErrEngagementAlreadyTerminal,
  eh.ErrRequestIDConflict,
  eh.ErrInternal,
 }
 if len(sentinels) != 14 {
  t.Fatalf("expected 14 sentinels, got %d", len(sentinels))
 }
 seen := map[eh.EngagementErrorCode]bool{}
 for _, s := range sentinels {
  if int32(s.Code) == 0 {
   t.Fatalf("sentinel has zero (UNSPECIFIED) code: %+v", s)
  }
  if seen[s.Code] {
   t.Fatalf("duplicate sentinel code: %v", s.Code)
  }
  seen[s.Code] = true
 }
}

func TestClassifiers_TruthTable(t *testing.T) {
 cases := []struct {
  code      eh.EngagementErrorCode
  transient bool
  terminal  bool
  client    bool
  server    bool
 }{
  {eh.CodeRouteResolutionFailed, false, false, true, false},
  {eh.CodeJourneyVersionNotFound, false, false, true, false},
  {eh.CodeTelephonyNotAvailable, false, false, true, false},
  {eh.CodeVoiceProfileNotFound, false, false, true, false},
  {eh.CodeVoiceSessionRejected, false, false, true, false},
  {eh.CodeJourneyExecutionRejected, false, false, true, false},
  {eh.CodeRegistryUnavailable, true, false, false, true},
  {eh.CodeContactUnreachable, false, true, true, false},
  {eh.CodeCallEndedWithError, false, false, true, false},
  {eh.CodeOrgQuotaExceeded, false, true, true, false},
  {eh.CodeEngagementNotFound, false, true, true, false},
  {eh.CodeEngagementAlreadyTerminal, false, true, true, false},
  {eh.CodeRequestIDConflict, false, true, true, false},
  {eh.CodeInternal, true, false, false, true},
 }
 for _, c := range cases {
  e := &eh.Error{Code: c.code}
  if got := eh.IsTransient(e); got != c.transient {
   t.Errorf("IsTransient(%v) = %v, want %v", c.code, got, c.transient)
  }
  if got := eh.IsTerminal(e); got != c.terminal {
   t.Errorf("IsTerminal(%v) = %v, want %v", c.code, got, c.terminal)
  }
  if got := eh.IsClientError(e); got != c.client {
   t.Errorf("IsClientError(%v) = %v, want %v", c.code, got, c.client)
  }
  if got := eh.IsServerError(e); got != c.server {
   t.Errorf("IsServerError(%v) = %v, want %v", c.code, got, c.server)
  }
 }
}

func TestClassifiers_NonEngagementError(t *testing.T) {
 plain := errors.New("plain")
 if eh.IsTransient(plain) || eh.IsTerminal(plain) || eh.IsClientError(plain) || eh.IsServerError(plain) {
  t.Fatal("plain error should not be classified")
 }
 if eh.IsTransient(nil) || eh.IsTerminal(nil) || eh.IsClientError(nil) || eh.IsServerError(nil) {
  t.Fatal("nil should not be classified")
 }
}

func TestClassifiers_UnwrapsThroughWrap(t *testing.T) {
 e := &eh.Error{Code: eh.CodeRegistryUnavailable}
 wrapped := fmt.Errorf("rpc failed: %w", e)
 if !eh.IsTransient(wrapped) {
  t.Fatal("IsTransient should unwrap through fmt.Errorf wrap")
 }
}

func TestFromConnectError_HappyPath(t *testing.T) {
 downstream := "engagement_hub"
 proto := &engagementv1.EngagementError{
  Code:              engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_REQUEST_ID_CONFLICT,
  Message:           "duplicate request_id",
  DownstreamService: &downstream,
  Details:           map[string]string{"existing_engagement_id": "eng-123"},
 }
 connErr := connect.NewError(connect.CodeAlreadyExists, errors.New("dup"))
 detail, err := connect.NewErrorDetail(proto)
 if err != nil {
  t.Fatalf("NewErrorDetail: %v", err)
 }
 connErr.AddDetail(detail)

 got, ok := eh.FromConnectError(connErr)
 if !ok {
  t.Fatal("expected ok=true")
 }
 if got.Code != eh.CodeRequestIDConflict {
  t.Errorf("Code = %v, want %v", got.Code, eh.CodeRequestIDConflict)
 }
 if got.Message != "duplicate request_id" {
  t.Errorf("Message = %q", got.Message)
 }
 if got.DownstreamService != "engagement_hub" {
  t.Errorf("DownstreamService = %q", got.DownstreamService)
 }
 if got.Details["existing_engagement_id"] != "eng-123" {
  t.Errorf("Details lost: %+v", got.Details)
 }
 if !errors.Is(got, connErr) {
  t.Fatal("FromConnectError should preserve original error as cause")
 }
 if !errors.Is(got, eh.ErrRequestIDConflict) {
  t.Fatal("converted error should match sentinel via errors.Is")
 }
}

func TestFromConnectError_NoDetail(t *testing.T) {
 connErr := connect.NewError(connect.CodeUnavailable, errors.New("down"))
 if _, ok := eh.FromConnectError(connErr); ok {
  t.Fatal("expected ok=false when connect error has no EngagementError detail")
 }
}

func TestFromConnectError_PlainError(t *testing.T) {
 if _, ok := eh.FromConnectError(errors.New("plain")); ok {
  t.Fatal("expected ok=false for plain (non-connect) error")
 }
}

func TestFromConnectError_NilError(t *testing.T) {
 if _, ok := eh.FromConnectError(nil); ok {
  t.Fatal("expected ok=false for nil")
 }
}

func TestFromConnectError_UnwrapsThroughWrap(t *testing.T) {
 proto := &engagementv1.EngagementError{
  Code:    engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_INTERNAL,
  Message: "boom",
 }
 connErr := connect.NewError(connect.CodeInternal, errors.New("x"))
 detail, _ := connect.NewErrorDetail(proto)
 connErr.AddDetail(detail)
 wrapped := fmt.Errorf("wrap: %w", connErr)

 got, ok := eh.FromConnectError(wrapped)
 if !ok {
  t.Fatal("FromConnectError should unwrap to find *connect.Error")
 }
 if got.Code != eh.CodeInternal {
  t.Errorf("Code = %v, want %v", got.Code, eh.CodeInternal)
 }
}
```

- [ ] **Step 2: Run tests — expect build error**

```bash
cd clients/go/engagementhub
go test .
```

Expected: build error — `eh.Error`, `eh.NewError`, sentinels (`eh.ErrRequestIDConflict`, …), classifiers (`eh.IsTransient`, …), and `eh.FromConnectError` are undefined (red step — correct)

- [ ] **Step 3: Commit the test file**

```bash
git add clients/go/engagementhub/errors_test.go
git commit -m "test(sdk/errors): add failing tests for typed Error + sentinels + classifiers (#24)"
```

---

### Task 2: Implement errors.go (green)

**Files:**

- Create: `clients/go/engagementhub/errors.go`

- [ ] **Step 1: Create the implementation**

```go
// Package engagementhub provides the Engagement Hub Go SDK.
package engagementhub

import (
 "errors"
 "fmt"

 "connectrpc.com/connect"
 engagementv1 "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen/revocall/engagement/v1"
)

// EngagementErrorCode mirrors the wire-level error code enum from
// revocall.engagement.v1.EngagementError, re-exported so consumers never
// import internal/gen/... directly.
type EngagementErrorCode = engagementv1.EngagementErrorCode

// Short-form code constants for ergonomic comparison.
const (
 CodeRouteResolutionFailed     = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ROUTE_RESOLUTION_FAILED
 CodeJourneyVersionNotFound    = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_JOURNEY_VERSION_NOT_FOUND
 CodeTelephonyNotAvailable     = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_TELEPHONY_NOT_AVAILABLE
 CodeVoiceProfileNotFound      = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_VOICE_PROFILE_NOT_FOUND
 CodeVoiceSessionRejected      = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_VOICE_SESSION_REJECTED
 CodeJourneyExecutionRejected  = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_JOURNEY_EXECUTION_REJECTED
 CodeRegistryUnavailable       = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_REGISTRY_UNAVAILABLE
 CodeContactUnreachable        = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_CONTACT_UNREACHABLE
 CodeCallEndedWithError        = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_CALL_ENDED_WITH_ERROR
 CodeOrgQuotaExceeded          = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ORG_QUOTA_EXCEEDED
 CodeEngagementNotFound        = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ENGAGEMENT_NOT_FOUND
 CodeEngagementAlreadyTerminal = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_ENGAGEMENT_ALREADY_TERMINAL
 CodeRequestIDConflict         = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_REQUEST_ID_CONFLICT
 CodeInternal                  = engagementv1.EngagementErrorCode_ENGAGEMENT_ERROR_CODE_INTERNAL
)

// Error is the SDK's typed representation of revocall.engagement.v1.EngagementError.
// Returned by SDK RPC wrappers when the server attaches an EngagementError detail
// to a *connect.Error. Compare with errors.Is(err, ErrXxx) and classify with
// IsTransient / IsTerminal / IsClientError / IsServerError.
type Error struct {
 Code              EngagementErrorCode
 Message           string
 DownstreamService string
 Details           map[string]string
 cause             error
}

// NewError constructs an *Error with an optional underlying cause.
func NewError(code EngagementErrorCode, message string, cause error) *Error {
 return &Error{Code: code, Message: message, cause: cause}
}

// Error implements the error interface.
func (e *Error) Error() string {
 if e.DownstreamService != "" {
  return fmt.Sprintf("engagement_hub: %s: %s (downstream=%s)", e.Code, e.Message, e.DownstreamService)
 }
 return fmt.Sprintf("engagement_hub: %s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying cause, enabling errors.Is/As chains.
func (e *Error) Unwrap() error { return e.cause }

// Is matches another *Error by Code only, so sentinels work through fmt.Errorf
// wrapping. Returns false for non-*Error targets.
func (e *Error) Is(target error) bool {
 t, ok := target.(*Error)
 if !ok {
  return false
 }
 return e.Code == t.Code
}

// Sentinels — one per non-UNSPECIFIED EngagementErrorCode. Use with errors.Is.
var (
 ErrRouteResolutionFailed     = &Error{Code: CodeRouteResolutionFailed}
 ErrJourneyVersionNotFound    = &Error{Code: CodeJourneyVersionNotFound}
 ErrTelephonyNotAvailable     = &Error{Code: CodeTelephonyNotAvailable}
 ErrVoiceProfileNotFound      = &Error{Code: CodeVoiceProfileNotFound}
 ErrVoiceSessionRejected      = &Error{Code: CodeVoiceSessionRejected}
 ErrJourneyExecutionRejected  = &Error{Code: CodeJourneyExecutionRejected}
 ErrRegistryUnavailable       = &Error{Code: CodeRegistryUnavailable}
 ErrContactUnreachable        = &Error{Code: CodeContactUnreachable}
 ErrCallEndedWithError        = &Error{Code: CodeCallEndedWithError}
 ErrOrgQuotaExceeded          = &Error{Code: CodeOrgQuotaExceeded}
 ErrEngagementNotFound        = &Error{Code: CodeEngagementNotFound}
 ErrEngagementAlreadyTerminal = &Error{Code: CodeEngagementAlreadyTerminal}
 ErrRequestIDConflict         = &Error{Code: CodeRequestIDConflict}
 ErrInternal                  = &Error{Code: CodeInternal}
)

// IsTransient reports whether err carries a code the retry middleware should
// safely re-attempt: registry outage or internal server error.
func IsTransient(err error) bool {
 var e *Error
 if !errors.As(err, &e) {
  return false
 }
 switch e.Code {
 case CodeRegistryUnavailable, CodeInternal:
  return true
 }
 return false
}

// IsTerminal reports whether err carries a code that indicates retrying with
// the same inputs will never succeed.
func IsTerminal(err error) bool {
 var e *Error
 if !errors.As(err, &e) {
  return false
 }
 switch e.Code {
 case CodeEngagementNotFound,
  CodeEngagementAlreadyTerminal,
  CodeContactUnreachable,
  CodeRequestIDConflict,
  CodeOrgQuotaExceeded:
  return true
 }
 return false
}

// IsClientError reports whether err is caller-attributable (every code except
// the two server-side codes).
func IsClientError(err error) bool {
 var e *Error
 if !errors.As(err, &e) {
  return false
 }
 if int32(e.Code) == 0 {
  return false
 }
 switch e.Code {
 case CodeRegistryUnavailable, CodeInternal:
  return false
 }
 return true
}

// IsServerError reports whether err is server-attributable.
func IsServerError(err error) bool {
 var e *Error
 if !errors.As(err, &e) {
  return false
 }
 switch e.Code {
 case CodeRegistryUnavailable, CodeInternal:
  return true
 }
 return false
}

// FromConnectError extracts a typed *Error from err if it (or any error in its
// chain) is a *connect.Error carrying a revocall.engagement.v1.EngagementError
// detail. The original err is preserved as the *Error's cause for Unwrap().
// Returns (nil, false) when no matching detail is found.
func FromConnectError(err error) (*Error, bool) {
 var connectErr *connect.Error
 if !errors.As(err, &connectErr) {
  return nil, false
 }
 for _, detail := range connectErr.Details() {
  msg, valErr := detail.Value()
  if valErr != nil {
   continue
  }
  protoErr, ok := msg.(*engagementv1.EngagementError)
  if !ok {
   continue
  }
  e := &Error{
   Code:    protoErr.Code,
   Message: protoErr.Message,
   Details: protoErr.Details,
   cause:   err,
  }
  if protoErr.DownstreamService != nil {
   e.DownstreamService = *protoErr.DownstreamService
  }
  return e, true
 }
 return nil, false
}
```

- [ ] **Step 2: Run tests — all 12 must pass**

```bash
cd clients/go/engagementhub
go test . -v
```

Expected: 12 `--- PASS` lines, `PASS`, `ok  github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub`

- [ ] **Step 3: Run go vet**

```bash
cd clients/go/engagementhub
go vet ./...
```

Expected: no output

- [ ] **Step 4: Commit**

```bash
git add clients/go/engagementhub/errors.go
git commit -m "feat(sdk/errors): add typed Error + sentinels + classifiers + FromConnectError (#24)"
```

---

### Deferred

- RPC wrappers that invoke `FromConnectError` — T2-06 (#25), T2-07 (#26)
- Server-side helper that builds a `*connect.Error` from an `*Error`
- `Wrap(cause, code)` ergonomic constructor for external callers — `NewError` covers known callers in this story
