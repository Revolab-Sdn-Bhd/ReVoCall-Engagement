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
