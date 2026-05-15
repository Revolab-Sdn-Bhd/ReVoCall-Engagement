# T2-03: Connect-Go stub generation + SDK module skeleton

**Issue:** #22 | **Branch:** feat/22-connect-go-stubs | **Date:** 2026-05-15

## Brainstorm

### Problem

No Go consumer (admin-backend, outbound dispatcher, ai-handler) can compile against Engagement Hub without generated Connect-Go stubs. T2-03 produces those stubs and stands up the Go module that all SDK hand-written code (T2-04 onward) builds on top of.

Two proto packages need to be wired:

- `revocall.engagement.v1` — external service surface (T2-01)
- `revocall.engagement.internal.v1` — lifecycle notification surface (T2-02)

### Options considered

**A. buf managed mode with `go_package_prefix` override (chosen)**
buf v2's `managed` block overrides the `go_package` file option at generation time. The proto files' existing `go_package` values (pointing to `gen/go/...`, set in T2-01/T2-02) are superseded without touching those files. Generated import paths become `clients/go/engagementhub/internal/gen/...`. `paths=source_relative` maps proto source paths directly to output paths.

**B. Change proto `go_package` options**
Update the proto files from T2-01/T2-02 to point to `clients/go/engagementhub/internal/gen/...`. No managed override needed. Ruled out: modifies already-merged protos; managed mode is the correct buf v2 pattern for this exact scenario.

**C. Local protoc plugins**
Use local `protoc-gen-go` and `protoc-gen-connect-go` instead of remote BSR plugins. Ruled out: requires unversioned local installs, non-standard for this toolchain.

### Decision

Option A. `buf.gen.yaml` at repo root uses managed mode with:

- `go_package_prefix: github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen`
- `buf.build/protocolbuffers/go` (pinned) → `clients/go/engagementhub/internal/gen`, `paths=source_relative`
- `buf.build/connectrpc/go` (pinned) → `clients/go/engagementhub/internal/gen`, `paths=source_relative`

SDK module at `clients/go/engagementhub/go.mod`:

- Module path: `github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub`
- Minimal direct deps: `connectrpc.com/connect`, `google.golang.org/protobuf`
- `go mod tidy` resolves transitive deps

gRPC-only mode is enforced at connection time in T2-04 (`connect.WithGRPC()`), not in stub generation. The stubs are protocol-agnostic.

Story doc path references corrected: T2-03 acceptance criteria and T2-12 use `clients/go/engagementhub/`, not `pkg/engagementhub/` (which was a naming inconsistency in the generated story docs).

## Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire up `buf generate` to produce Connect-Go stubs from both proto packages and stand up `clients/go/engagementhub` as a buildable Go module with correct import paths.

**Architecture:** buf v2 managed mode overrides the `go_package_prefix` at generation time (proto files stay unchanged) routing both proto packages' output into `clients/go/engagementhub/internal/gen/`. A separate Go module at `clients/go/engagementhub/go.mod` declares two direct runtime deps; `go mod tidy` resolves transitive deps. No hand-written SDK code — that starts T2-04.

**Tech Stack:** buf 1.69.0 (`/opt/homebrew/bin/buf`), buf BSR remote plugins `buf.build/protocolbuffers/go:v1.36.11` + `buf.build/connectrpc/go:v1.19.2`, Go 1.26 (`/opt/homebrew/bin/go`), `connectrpc.com/connect v1.19.2`, `google.golang.org/protobuf v1.36.11`

### Design decisions

- SDK module lives at `clients/go/engagementhub/` (PRD §8.1 canonical path; `pkg/engagementhub/` in the story doc acceptance criteria was a naming inconsistency corrected here).
- buf managed mode with `go_package_prefix` override routes generated import paths to `clients/go/engagementhub/internal/gen/...` without touching the proto files set in T2-01/T2-02.
- `paths=source_relative` maps proto source paths to output paths directly.
- gRPC-only enforcement deferred to T2-04 (`connect.WithGRPC()` at connection construction); stub generation is protocol-agnostic.
- Two direct deps only: `connectrpc.com/connect` (runtime for Connect stubs) and `google.golang.org/protobuf` (proto runtime + WKT). No `google.golang.org/genproto` needed — WKT are in the `protobuf` module.
- connectrpc/go v1.19.x uses sub-package layout: Connect stubs land in `engagementv1connect/` and `internalv1connect/` sub-packages (not flat `_connect.go`).

### File map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `buf.gen.yaml` | buf managed-mode codegen config |
| Create | `clients/go/engagementhub/go.mod` | SDK Go module declaration |
| Delete | `clients/go/engagementhub/.gitkeep` | Placeholder from T1-01 scaffold |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/v1/*.pb.go` | External proto messages |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/v1/engagementv1connect/service.connect.go` | External Connect-Go client/server stubs |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/service.pb.go` | Internal proto messages |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/internalv1connect/service.connect.go` | Internal Connect-Go stubs |
| Generated | `clients/go/engagementhub/go.sum` | Produced by `go mod tidy` |
| Modify | `justfile` | Add `buf-gen` recipe |
| Modify | `docs/stories/T2-03-connect-go-stubs-sdk-skeleton.md` | This file |

### Task 1: Create buf.gen.yaml and verify buf lint

**Files:**

- Create: `buf.gen.yaml`

- [x] **Step 1: Create buf.gen.yaml at repo root**

```yaml
# buf.gen.yaml
# Regenerate stubs: just buf-gen
version: v2
managed:
  enabled: true
  override:
    - file_option: go_package_prefix
      value: github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen
plugins:
  - remote: buf.build/protocolbuffers/go:v1.36.11
    out: clients/go/engagementhub/internal/gen
    opt: paths=source_relative
  - remote: buf.build/connectrpc/go:v1.19.2
    out: clients/go/engagementhub/internal/gen
    opt: paths=source_relative
```

`go_package_prefix` overrides the module path component of `go_package` while preserving the package name alias (e.g. `engagementv1`). `paths=source_relative` maps each proto's path relative to the buf module root (`proto/`) directly to the output tree.

- [x] **Step 2: Verify buf lint still passes**

```bash
/opt/homebrew/bin/buf lint
```

Expected: no output (exit code 0). If errors appear they are pre-existing and unrelated to buf.gen.yaml — do not fix them here.

- [x] **Step 3: Commit**

```bash
git add buf.gen.yaml
git commit -m "build: add buf.gen.yaml — Connect-Go managed-mode config (#22)"
```

---

### Task 2: Run buf generate and commit generated stubs

**Files:**

- Generated: `clients/go/engagementhub/internal/gen/` (entire subtree)

- [x] **Step 1: Run buf generate**

```bash
/opt/homebrew/bin/buf generate
```

Expected: exits 0, no errors. buf downloads the remote plugins on first run (may take a few seconds).

- [x] **Step 2: Verify output directory structure**

```bash
find clients/go/engagementhub/internal/gen -type f | sort
```

Expected (10 files; connectrpc/go v1.19.x places stubs in `*connect/` sub-packages):

```
clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/internalv1connect/service.connect.go
clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/service.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/control.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/engagement.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/engagementv1connect/service.connect.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/errors.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/proxied.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/query.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/service.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/watch.pb.go
```

- [x] **Step 3: Spot-check import path in the generated Connect stub**

```bash
head -10 clients/go/engagementhub/internal/gen/revocall/engagement/v1/engagementv1connect/service.connect.go
```

The `package` line must be `package engagementv1connect` and imports must begin with `github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen/...`.

- [x] **Step 4: Commit generated stubs**

```bash
git add clients/go/engagementhub/internal/gen/
git commit -m "build: generate Connect-Go stubs into clients/go/engagementhub/internal/gen (#22)"
```

---

### Task 3: Create go.mod, run go mod tidy, verify build

**Files:**

- Delete: `clients/go/engagementhub/.gitkeep`
- Create: `clients/go/engagementhub/go.mod`
- Generated: `clients/go/engagementhub/go.sum`

- [x] **Step 1: Remove the .gitkeep placeholder**

```bash
rm clients/go/engagementhub/.gitkeep
```

- [x] **Step 2: Create clients/go/engagementhub/go.mod**

```
module github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub

go 1.26

require (
    connectrpc.com/connect v1.19.2
    google.golang.org/protobuf v1.36.11
)
```

`connectrpc.com/connect` provides the Connect-Go runtime types. `google.golang.org/protobuf` provides the proto message runtime and all WKT types (Timestamp, Duration, FieldMask).

- [x] **Step 3: Run go mod tidy**

```bash
cd clients/go/engagementhub && go mod tidy && cd -
```

Expected: exits 0, produces `go.sum`. Note: `github.com/google/go-cmp` appears in `go.sum` as a transitive test dep of `google.golang.org/protobuf` — this is expected and correct.

- [x] **Step 4: Verify the module builds**

```bash
cd clients/go/engagementhub && go build ./... && cd -
```

Expected: exits 0, no compiler errors. This is the primary acceptance gate for this task.

- [x] **Step 5: Commit**

```bash
git add clients/go/engagementhub/go.mod clients/go/engagementhub/go.sum
git rm clients/go/engagementhub/.gitkeep
git commit -m "feat: stand up clients/go/engagementhub Go module skeleton (#22)"
```

---

### Task 4: Add buf-gen recipe to justfile

**Files:**

- Modify: `justfile`

- [x] **Step 1: Add the recipe**

Append after the last existing recipe in `justfile`:

```just
# Regenerate Connect-Go stubs from both proto packages (requires buf 1.69+)
buf-gen:
    buf generate
```

- [x] **Step 2: Verify the recipe works and is idempotent**

```bash
just buf-gen
git diff --stat clients/go/engagementhub/internal/gen/
```

Expected: empty diff — buf generate is deterministic.

- [x] **Step 3: Commit**

```bash
git add justfile
git commit -m "build: add buf-gen recipe to justfile (#22)"
```

---

### Task 5: Correct story doc path references

**Files:**

- Modify: `docs/stories/T2-03-connect-go-stubs-sdk-skeleton.md` (this file)
- Modify: `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-03-connect-go-stubs-sdk-skeleton.md`
- Modify: `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-12-e2e-integration-v1-release.md`

- [x] **Step 1: Merge implementation plan into story doc**

Merged full plan content from `docs/superpowers/plans/2026-05-15-connect-go-stubs-sdk-skeleton.md` into this file. Separate plan file deleted.

- [x] **Step 2: Correct external T2-03 story doc acceptance criteria**

Updated `pkg/engagementhub` → `clients/go/engagementhub/` in context, acceptance criteria, and context line.

- [x] **Step 3: Correct external T2-12 story doc**

Updated `pkg/engagementhub/VERSIONING.md` → `clients/go/engagementhub/VERSIONING.md`.

- [x] **Step 4: Commit**

```bash
git add docs/stories/T2-03-connect-go-stubs-sdk-skeleton.md
git commit -m "docs: update story doc #22 — merge implementation plan, correct clients/go/ path"
```

The external docs are outside the worktree and not tracked by this branch's git; update them in place (no commit needed for those files).

### Deferred

- Hand-written SDK code (`client.go`, `options.go`, middleware) — T2-04 onward
- gRPC-only enforcement via `connect.WithGRPC()` — T2-04
- `enghubtest` fake client — T2-11
