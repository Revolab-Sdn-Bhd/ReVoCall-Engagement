# T2-03: Connect-Go Stub Generation + SDK Module Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire up `buf generate` to produce Connect-Go stubs from both proto packages and stand up `clients/go/engagementhub` as a buildable Go module with correct import paths.

**Architecture:** buf v2 managed mode overrides the `go_package_prefix` at generation time (proto files stay unchanged) routing both proto packages' output into `clients/go/engagementhub/internal/gen/`. A separate Go module at `clients/go/engagementhub/go.mod` declares two direct runtime deps; `go mod tidy` resolves transitive deps. No hand-written SDK code — that starts T2-04.

**Tech Stack:** buf 1.69.0 (`/opt/homebrew/bin/buf`), buf BSR remote plugins `buf.build/protocolbuffers/go:v1.36.11` + `buf.build/connectrpc/go:v1.19.2`, Go 1.26 (`/opt/homebrew/bin/go`), `connectrpc.com/connect v1.19.2`, `google.golang.org/protobuf v1.36.11`

**Worktree:** `/Users/chunzhe/Projects/ReVoCall-Engagement.t2-03` (branch `feat/22-connect-go-stubs`)
All commands run from the worktree root unless stated otherwise.

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `buf.gen.yaml` | buf managed-mode codegen config |
| Create | `clients/go/engagementhub/go.mod` | SDK Go module declaration |
| Delete | `clients/go/engagementhub/.gitkeep` | Placeholder from T1-01 scaffold |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/v1/*.pb.go` | External proto messages |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/v1/service_connect.go` | External Connect-Go client/server stubs |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/service.pb.go` | Internal proto messages |
| Generated | `clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/service_connect.go` | Internal Connect-Go stubs |
| Generated | `clients/go/engagementhub/go.sum` | Produced by `go mod tidy` |
| Modify | `justfile` | Add `buf-gen` recipe |
| Modify | `docs/stories/T2-03-connect-go-stubs-sdk-skeleton.md` | Add implementation plan section |
| Modify | `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-03-connect-go-stubs-sdk-skeleton.md` | Correct `pkg/` → `clients/go/` in acceptance criteria |
| Modify | `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-12-e2e-integration-v1-release.md` | Correct `pkg/engagementhub/VERSIONING.md` path |

---

## Task 1: Create buf.gen.yaml and verify buf lint

**Files:**
- Create: `buf.gen.yaml`

- [ ] **Step 1: Create buf.gen.yaml at repo root**

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

`go_package_prefix` overrides the module path component of `go_package` while preserving the package name alias (e.g. `engagementv1`). `paths=source_relative` maps each proto's path relative to the buf module root (`proto/`) directly to the output tree — so `proto/revocall/engagement/v1/service.proto` → `clients/go/engagementhub/internal/gen/revocall/engagement/v1/service_connect.go`.

- [ ] **Step 2: Verify buf lint still passes**

```bash
/opt/homebrew/bin/buf lint
```

Expected: no output (exit code 0). If errors appear they are pre-existing and unrelated to buf.gen.yaml — do not fix them here.

- [ ] **Step 3: Commit**

```bash
git add buf.gen.yaml
git commit -m "build: add buf.gen.yaml — Connect-Go managed-mode config (#22)"
```

---

## Task 2: Run buf generate and commit generated stubs

**Files:**
- Generated: `clients/go/engagementhub/internal/gen/` (entire subtree)

- [ ] **Step 1: Run buf generate**

```bash
/opt/homebrew/bin/buf generate
```

Expected: exits 0, no errors. buf downloads the remote plugins on first run (may take a few seconds). If you see `plugin "buf.build/protocolbuffers/go:v1.36.11" not found`, the exact patch version may not be published yet — retry with the closest published version:

```bash
# Find available versions
/opt/homebrew/bin/buf registry plugin list --remote buf.build --owner protocolbuffers
# Then update buf.gen.yaml to use the closest available version, e.g. v1.36.6
```

Similarly for connectrpc/go if needed.

- [ ] **Step 2: Verify output directory structure**

```bash
find clients/go/engagementhub/internal/gen -type f | sort
```

Expected (10 files):
```
clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/service.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/internal/v1/service_connect.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/control.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/engagement.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/errors.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/proxied.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/query.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/service.pb.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/service_connect.go
clients/go/engagementhub/internal/gen/revocall/engagement/v1/watch.pb.go
```

Only `service.proto` files generate `_connect.go` (they are the only files with a `service` definition). All proto files get a `.pb.go`.

- [ ] **Step 3: Spot-check import path in the generated Connect stub**

```bash
head -10 clients/go/engagementhub/internal/gen/revocall/engagement/v1/service_connect.go
```

The `package` line must be `package engagementv1` and the file must import from paths beginning with `github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub/internal/gen/...` — NOT from `gen/go/...`. If you see `gen/go/` the managed override is not working; re-check buf.gen.yaml.

- [ ] **Step 4: Commit generated stubs**

```bash
git add clients/go/engagementhub/internal/gen/
git commit -m "build: generate Connect-Go stubs into clients/go/engagementhub/internal/gen (#22)"
```

---

## Task 3: Create go.mod, run go mod tidy, verify build

**Files:**
- Delete: `clients/go/engagementhub/.gitkeep`
- Create: `clients/go/engagementhub/go.mod`
- Generated: `clients/go/engagementhub/go.sum`

- [ ] **Step 1: Remove the .gitkeep placeholder**

```bash
rm clients/go/engagementhub/.gitkeep
```

- [ ] **Step 2: Create clients/go/engagementhub/go.mod**

```
module github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/clients/go/engagementhub

go 1.26

require (
	connectrpc.com/connect v1.19.2
	google.golang.org/protobuf v1.36.11
)
```

`connectrpc.com/connect` provides the Connect-Go runtime types referenced in the generated `_connect.go` stubs. `google.golang.org/protobuf` provides the proto message runtime and all WKT types (Timestamp, Duration, FieldMask) referenced in the `.pb.go` files.

- [ ] **Step 3: Run go mod tidy**

```bash
cd clients/go/engagementhub && go mod tidy && cd -
```

Expected: exits 0, produces `go.sum`. If you see `no required module provides package X`, the generated stubs import a package not covered by the two direct deps — add the missing dep to go.mod and re-run.

- [ ] **Step 4: Verify the module builds**

```bash
cd clients/go/engagementhub && go build ./... && cd -
```

Expected: exits 0, no compiler errors. This is the primary acceptance gate for this task. If you see `undefined:` errors, the generated code references a type that the declared deps don't satisfy — check that the buf plugin versions align with the go.mod versions.

- [ ] **Step 5: Commit**

```bash
git add clients/go/engagementhub/go.mod clients/go/engagementhub/go.sum
git rm clients/go/engagementhub/.gitkeep
git commit -m "feat: stand up clients/go/engagementhub Go module skeleton (#22)"
```

---

## Task 4: Add buf-gen recipe to justfile

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Add the recipe**

Open `justfile` and append after the last existing recipe:

```just
# Regenerate Connect-Go stubs from both proto packages (requires buf 1.69+)
buf-gen:
    buf generate
```

- [ ] **Step 2: Verify the recipe works and is idempotent**

```bash
just buf-gen
git diff --stat clients/go/engagementhub/internal/gen/
```

Expected: empty diff — buf generate is deterministic; re-running produces identical output. If any files show as changed, inspect with `git diff clients/go/engagementhub/internal/gen/` to understand what differs before committing the updated output.

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "build: add buf-gen recipe to justfile (#22)"
```

---

## Task 5: Correct story doc path references and finalise in-repo story doc

**Files:**
- Modify: `docs/stories/T2-03-connect-go-stubs-sdk-skeleton.md` (in-repo)
- Modify: `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-03-connect-go-stubs-sdk-skeleton.md`
- Modify: `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-12-e2e-integration-v1-release.md`

- [ ] **Step 1: Update in-repo story doc — add implementation plan section**

In `docs/stories/T2-03-connect-go-stubs-sdk-skeleton.md`, replace `## Implementation plan\n\n_To be added after writing-plans._` with:

```markdown
## Implementation plan

### Design decisions

- SDK module lives at `clients/go/engagementhub/` (PRD §8.1 canonical path; `pkg/engagementhub/` in the story doc acceptance criteria was a naming inconsistency corrected here).
- buf managed mode with `go_package_prefix` override routes generated import paths to `clients/go/engagementhub/internal/gen/...` without touching the proto files set in T2-01/T2-02.
- `paths=source_relative` maps proto source paths to output paths directly.
- gRPC-only enforcement deferred to T2-04 (`connect.WithGRPC()` at connection construction); stub generation is protocol-agnostic.
- Two direct deps only: `connectrpc.com/connect` (runtime for Connect stubs) and `google.golang.org/protobuf` (proto runtime + WKT). No `google.golang.org/genproto` needed — WKT are in the `protobuf` module.

### Tasks

1. Create `buf.gen.yaml` — managed mode + plugin config; verify `buf lint`
2. Run `buf generate` — produce 10 generated files; verify import paths
3. Create `clients/go/engagementhub/go.mod`; run `go mod tidy`; verify `go build ./...`
4. Add `buf-gen` recipe to `justfile`; verify idempotent regeneration
5. Correct `pkg/engagementhub/` → `clients/go/engagementhub/` in external story docs; finalise this file

### Deferred

- Hand-written SDK code (`client.go`, `options.go`, middleware) — T2-04 onward
- gRPC-only enforcement via `connect.WithGRPC()` — T2-04
- `enghubtest` fake client — T2-11
```

- [ ] **Step 2: Correct external T2-03 story doc acceptance criteria**

In `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-03-connect-go-stubs-sdk-skeleton.md`:

Replace:
```
- Stubs output to `pkg/engagementhub/internal/gen/`
```
With:
```
- Stubs output to `clients/go/engagementhub/internal/gen/`
```

Replace:
```
- `pkg/engagementhub/go.mod` with minimal dependency set (protobuf, Connect-Go, Google WKT)
```
With:
```
- `clients/go/engagementhub/go.mod` with minimal dependency set (protobuf, Connect-Go); WKT provided by `google.golang.org/protobuf`, no separate genproto needed
```

Also update the context line:
```
Wires up `buf generate` to produce Connect-Go stubs from the external + internal proto packages, and stands up the `pkg/engagementhub` Go module
```
Replace `pkg/engagementhub` with `clients/go/engagementhub`.

- [ ] **Step 3: Correct external T2-12 story doc**

In `/Users/chunzhe/Projects/docs/2026-05-14-engagement-hub-stories/T2-12-e2e-integration-v1-release.md`:

Replace:
```
- **Versioning**: `pkg/engagementhub/VERSIONING.md` documenting semver policy, module-path-v2 strategy for future breaking changes
```
With:
```
- **Versioning**: `clients/go/engagementhub/VERSIONING.md` documenting semver policy, module-path-v2 strategy for future breaking changes
```

- [ ] **Step 4: Commit all story doc corrections**

```bash
git add docs/stories/T2-03-connect-go-stubs-sdk-skeleton.md
git commit -m "docs: update story doc #22 — add implementation plan, correct clients/go/ path"
```

The external docs are outside the worktree and not tracked by this branch's git; update them in place (no commit needed for those files).
