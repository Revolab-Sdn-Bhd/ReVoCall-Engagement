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
- A `Wrap(cause error, code) *Error` helper — sentinels + `&Error{Code: …, Message: …, cause: …}` literal construction covers known cases; add later if call sites demand it
