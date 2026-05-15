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

_To be added after writing-plans._
