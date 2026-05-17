# T2-10: Shared UUIDv5 idempotency-key derivation package

**Issue:** #29 | **Branch:** feat/29-shared-uuidv5-idempotency | **Date:** 2026-05-17

## Brainstorm

### Problem

The EH `StartEngagement` RPC uses `request_id` for idempotency: the same `(batch_id, contact_number, attempt_number)` tuple must always produce the same UUID so that retries are deduplicated and `REQUEST_ID_CONFLICT` detection works correctly. Both the Go `BatchTracker` (T2-09) and the future Rust outbound dispatcher (T6) must compute identical IDs — requiring the derivation logic to live in a single canonical location rather than being re-implemented in each caller.

### Options considered

**A. Sub-package inside the existing SDK module (chosen)**

Place `shared/idempotency/` inside `clients/go/engagementhub/` so it shares the same Go module. BatchTracker imports it with a local path; no new module management overhead. Rust callers document the mirrored signature but implement it themselves (T6 scope).

**B. Separate Go module (`clients/go/shared/`)**

Own `go.mod`, independently versioned. Only justified if non-SDK consumers (e.g. admin-backend without the full SDK) need this utility. No such caller exists today; adds module management cost for no gain.

**C. Stdlib-only implementation (no new dependency)**

Manual SHA-1 + RFC 4122 byte layout. Zero new dep, but ~30 lines of fiddly bit-twiddling that is trivially covered by `github.com/google/uuid`. Not worth it.

### Decision

Sub-package inside the SDK module using `github.com/google/uuid` for UUIDv5. Package lives at `clients/go/engagementhub/shared/idempotency/`; import path is `github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/shared/idempotency`. The PRD's illustrative path (`github.com/revolab/revocall-engagement/shared/idempotency`) is not canonical — the committed `go.mod` form is authoritative.

The fixed namespace `NAMESPACE_OUTBOUND` is a purpose-generated UUID (`ba40c89b-d320-47cd-aa7c-c05c3b24dd6a`) — not a standard RFC 4122 namespace — to avoid accidental collisions with other UUIDv5 uses. The namespace value is exported as `NamespaceOutbound` so tests and the Rust mirror can reference it explicitly.

`attempt_number` is passed as a plain decimal string (`strconv.Itoa(n)` in Go, `n.to_string()` in Rust) — no zero-padding. This format is part of the cross-language stability contract and must not change without a coordinated migration.

Empty inputs are a programming error. `DeriveRequestID` panics if any argument is empty string. Keeps the signature clean (`string` return only); BatchTracker is the only caller and must never pass empty values.

Rust mirror signature is documented in `docs/rust-mirror/idempotency.md` (no Rust implementation in this story — that is T6 scope).

## Implementation plan

_To be added in Phase 3 (writing-plans)._
