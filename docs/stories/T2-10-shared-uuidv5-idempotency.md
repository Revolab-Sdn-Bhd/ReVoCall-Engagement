# T2-10: Shared UUIDv5 idempotency-key derivation package

**Issue:** #29 | **Branch:** feat/29-shared-uuidv5-idempotency | **Date:** 2026-05-17

## Brainstorm

### Problem

The EH `StartEngagement` RPC uses `request_id` for idempotency: the same `(batch_id, contact_number, attempt_number)` tuple must always produce the same UUID so that retries are deduplicated and `REQUEST_ID_CONFLICT` detection works correctly. Both the Go `BatchTracker` (T2-09) and the future Rust outbound dispatcher (T6) must compute identical IDs — requiring the derivation logic to live in a single canonical location rather than being re-implemented in each caller.

### Options considered

#### A. Sub-package inside the existing SDK module (chosen)

Place `shared/idempotency/` inside `clients/go/engagementhub/` so it shares the same Go module. BatchTracker imports it with a local path; no new module management overhead. Rust callers document the mirrored signature but implement it themselves (T6 scope).

**B. Separate Go module (`clients/go/shared/`)**

Own `go.mod`, independently versioned. Only justified if non-SDK consumers (e.g. admin-backend without the full SDK) need this utility. No such caller exists today; adds module management cost for no gain.

#### C. Stdlib-only implementation (no new dependency)

Manual SHA-1 + RFC 4122 byte layout. Zero new dep, but ~30 lines of fiddly bit-twiddling that is trivially covered by `github.com/google/uuid`. Not worth it.

### Decision

Sub-package inside the SDK module using `github.com/google/uuid` for UUIDv5. Package lives at `clients/go/engagementhub/shared/idempotency/`; import path is `github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/shared/idempotency`. The PRD's illustrative path (`github.com/revolab/revocall-engagement/shared/idempotency`) is not canonical — the committed `go.mod` form is authoritative.

The fixed namespace `NAMESPACE_OUTBOUND` is a purpose-generated UUID (`ba40c89b-d320-47cd-aa7c-c05c3b24dd6a`) — not a standard RFC 4122 namespace — to avoid accidental collisions with other UUIDv5 uses. The namespace value is exported as `NamespaceOutbound` so tests and the Rust mirror can reference it explicitly.

`attempt_number` is passed as a plain decimal string (`strconv.Itoa(n)` in Go, `n.to_string()` in Rust) — no zero-padding. This format is part of the cross-language stability contract and must not change without a coordinated migration.

Empty inputs are a programming error. `DeriveRequestID` panics if any argument is empty string. Keeps the signature clean (`string` return only); BatchTracker is the only caller and must never pass empty values.

Rust mirror signature is documented in `docs/rust-mirror/idempotency.md` (no Rust implementation in this story — that is T6 scope).

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `shared/idempotency` sub-package to the Go SDK that exports `DeriveRequestID` for deterministic UUIDv5 request_id derivation, plus a Rust mirror signature doc.

**Architecture:** Single Go file in a new sub-package at `clients/go/engagementhub/shared/idempotency/`. Exports one function and one namespace constant. No new module — same `go.mod`. Rust mirror is documentation only in this story.

**Tech Stack:** Go 1.26, `github.com/google/uuid` (latest v1.x), stdlib `go test`.

### Design decisions

- Sub-package in same module — BatchTracker (T2-09) imports locally; no separate module versioning overhead
- `NamespaceOutbound` exported — tests and Rust mirror reference the exact byte value
- `attempt_number` as plain decimal string — `strconv.Itoa(n)` / `n.to_string()`, no zero-padding; cross-language stability contract, must not change without coordinated migration
- Panic on empty inputs — programming error, not a runtime condition; keeps signature clean (`string` return only)
- Known-value pin test — guards against silent output changes from dependency upgrades; Rust impl must match these values

### File map

| Action | Path | Responsibility |
| -------- | ------ | ---------------- |
| Modify | `clients/go/engagementhub/go.mod` | Add `github.com/google/uuid` require |
| Modify | `clients/go/engagementhub/go.sum` | Updated by `go get` |
| Create | `clients/go/engagementhub/shared/idempotency/request_id.go` | `DeriveRequestID` + `NamespaceOutbound` |
| Create | `clients/go/engagementhub/shared/idempotency/request_id_test.go` | All tests |
| Create | `docs/rust-mirror/idempotency.md` | Rust mirror signature + namespace constant + known-value pins |

---

### Task 1: Add github.com/google/uuid dependency

**Files:**

- Modify: `clients/go/engagementhub/go.mod`
- Modify: `clients/go/engagementhub/go.sum`

- [ ] **Step 1: Add the uuid package**

```bash
cd clients/go/engagementhub
go get github.com/google/uuid@latest
```

Expected: `go: added github.com/google/uuid v1.x.x`

- [ ] **Step 2: Verify go.mod updated**

```bash
grep uuid clients/go/engagementhub/go.mod
```

Expected: line containing `github.com/google/uuid v1.`

- [ ] **Step 3: Commit**

```bash
git add clients/go/engagementhub/go.mod clients/go/engagementhub/go.sum
git commit -m "chore(sdk): add github.com/google/uuid dependency (#29)"
```

---

### Task 2: Write failing tests (red)

**Files:**

- Create: `clients/go/engagementhub/shared/idempotency/request_id_test.go`

- [ ] **Step 1: Create the test file**

```go
package idempotency_test

import (
 "regexp"
 "testing"

 "github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/shared/idempotency"
)

var uuidRE = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)

func TestDeriveRequestID_Determinism(t *testing.T) {
 first := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
 for i := 0; i < 1000; i++ {
  got := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
  if got != first {
   t.Fatalf("non-deterministic: iteration %d got %s, want %s", i, got, first)
  }
 }
}

func TestDeriveRequestID_CrossAttemptCollision(t *testing.T) {
 a1 := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
 a2 := idempotency.DeriveRequestID("batch-123", "+60126013446", "2")
 if a1 == a2 {
  t.Fatalf("attempt 1 and attempt 2 produced the same UUID: %s", a1)
 }
}

func TestDeriveRequestID_CrossContactCollision(t *testing.T) {
 c1 := idempotency.DeriveRequestID("batch-123", "+60126013446", "1")
 c2 := idempotency.DeriveRequestID("batch-123", "+60126013447", "1")
 if c1 == c2 {
  t.Fatalf("different contacts produced the same UUID: %s", c1)
 }
}

func TestDeriveRequestID_RFC4122Format(t *testing.T) {
 got := idempotency.DeriveRequestID("batch-abc", "+60126013446", "1")
 if !uuidRE.MatchString(got) {
  t.Fatalf("output %q is not lowercase hyphenated RFC 4122 UUID", got)
 }
}

func TestDeriveRequestID_KnownValue(t *testing.T) {
 // Pin against a computed-once value. If this fails after a dep upgrade,
 // the derivation contract has changed — investigate before proceeding.
 const want = "03518426-c533-5d8f-bbb9-f8ad0c139ffb"
 got := idempotency.DeriveRequestID("batch-abc", "+60126013446", "1")
 if got != want {
  t.Fatalf("known-value regression: got %s, want %s", got, want)
 }
}

func TestDeriveRequestID_PanicsOnEmptyBatchID(t *testing.T) {
 defer func() {
  if r := recover(); r == nil {
   t.Fatal("expected panic for empty batchID, got none")
  }
 }()
 idempotency.DeriveRequestID("", "+60126013446", "1")
}

func TestDeriveRequestID_PanicsOnEmptyContactNumber(t *testing.T) {
 defer func() {
  if r := recover(); r == nil {
   t.Fatal("expected panic for empty contactNumber, got none")
  }
 }()
 idempotency.DeriveRequestID("batch-123", "", "1")
}

func TestDeriveRequestID_PanicsOnEmptyAttemptNumber(t *testing.T) {
 defer func() {
  if r := recover(); r == nil {
   t.Fatal("expected panic for empty attemptNumber, got none")
  }
 }()
 idempotency.DeriveRequestID("batch-123", "+60126013446", "")
}
```

- [ ] **Step 2: Run — expect build error (no implementation yet)**

```bash
cd clients/go/engagementhub
go test ./shared/idempotency/...
```

Expected: build error — package has no non-test Go files yet (red step — correct)

- [ ] **Step 3: Commit the test file**

```bash
git add clients/go/engagementhub/shared/idempotency/request_id_test.go
git commit -m "test(sdk/idempotency): add failing tests for DeriveRequestID (#29)"
```

---

### Task 3: Implement DeriveRequestID (green)

**Files:**

- Create: `clients/go/engagementhub/shared/idempotency/request_id.go`

- [ ] **Step 1: Create the implementation**

```go
package idempotency

import "github.com/google/uuid"

// NamespaceOutbound is the fixed UUIDv5 namespace for outbound dispatcher idempotency keys.
//
// Rust mirror:
//
// use uuid::{uuid, Uuid};
// const NAMESPACE_OUTBOUND: Uuid = uuid!("ba40c89b-d320-47cd-aa7c-c05c3b24dd6a");
var NamespaceOutbound = uuid.MustParse("ba40c89b-d320-47cd-aa7c-c05c3b24dd6a")

// DeriveRequestID returns a deterministic UUIDv5 request_id for the given outbound
// campaign attempt. Derivation: UUIDv5(NamespaceOutbound, batchID+":"+contactNumber+":"+attemptNumber).
// attemptNumber must be the plain decimal string of the attempt integer ("1", "2", ...).
// Inputs must not contain ":". Panics if any argument is empty.
func DeriveRequestID(batchID, contactNumber, attemptNumber string) string {
 if batchID == "" || contactNumber == "" || attemptNumber == "" {
  panic("idempotency.DeriveRequestID: all arguments must be non-empty")
 }
 return uuid.NewSHA1(NamespaceOutbound, []byte(batchID+":"+contactNumber+":"+attemptNumber)).String()
}
```

- [ ] **Step 2: Run tests — all 8 must pass**

```bash
cd clients/go/engagementhub
go test ./shared/idempotency/... -v
```

Expected: 8 tests, all `--- PASS`

- [ ] **Step 3: Commit**

```bash
git add clients/go/engagementhub/shared/idempotency/request_id.go
git commit -m "feat(sdk/idempotency): add DeriveRequestID UUIDv5 package (#29)"
```

---

### Task 4: Rust mirror documentation

**Files:**

- Create: `docs/rust-mirror/idempotency.md`

- [ ] **Step 1: Create `docs/rust-mirror/idempotency.md` with this content**

````markdown
# Rust mirror: idempotency key derivation

Go canonical: `clients/go/engagementhub/shared/idempotency`
Rust implementation scope: T6-04 (#56)

## Namespace constant

```rust
use uuid::{uuid, Uuid};

const NAMESPACE_OUTBOUND: Uuid = uuid!("ba40c89b-d320-47cd-aa7c-c05c3b24dd6a");
```

## Function signature

```rust
/// Derives a deterministic UUIDv5 request_id for the given outbound campaign attempt.
///
/// `attempt_number` must be the plain decimal string of the attempt integer ("1", "2", ...).
/// Inputs must not contain ":". Panics if any argument is empty.
pub fn derive_request_id(batch_id: &str, contact_number: &str, attempt_number: &str) -> String {
    assert!(
        !batch_id.is_empty() && !contact_number.is_empty() && !attempt_number.is_empty(),
        "derive_request_id: all arguments must be non-empty"
    );
    let data = format!("{}:{}:{}", batch_id, contact_number, attempt_number);
    Uuid::new_v5(&NAMESPACE_OUTBOUND, data.as_bytes()).to_string()
}
```

Required: `uuid = { version = "1", features = ["v5"] }`

## Known-value pins

The Rust implementation must produce identical UUIDs for identical inputs:

| batch_id | contact_number | attempt_number | Expected UUID |
|----------|----------------|----------------|---------------|
| `batch-abc` | `+60126013446` | `1` | `03518426-c533-5d8f-bbb9-f8ad0c139ffb` |
| `batch-abc` | `+60126013446` | `2` | `092e314e-4c2b-59d8-9991-1c438df81e2e` |
| `batch-abc` | `+60126013447` | `1` | `49443967-f52d-512f-9934-03269b7e401c` |
````

- [ ] **Step 2: Commit**

```bash
git add docs/rust-mirror/idempotency.md
git commit -m "docs(rust-mirror): add idempotency UUIDv5 mirror signature for T6 (#29)"
```

---

### Deferred

- Rust implementation of `derive_request_id` — T6-04 (#56)
- BatchTracker auto-derivation when `request_id` omitted — T2-09 (#28)
