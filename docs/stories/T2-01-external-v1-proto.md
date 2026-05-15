# T2-01: External v1 proto

**Issue:** #20 | **Branch:** feat/20-external-proto | **Date:** 2026-05-15

## Brainstorm

### Problem

Every downstream consumer (admin-backend Track A, outbound Track B, ai-handler T4) needs a stable `.proto` contract to compile against before T1-03+ can wire real adapters. Defining it in T2-01 decouples proto evolution from service implementation.

### Options considered

N/A — buf-managed proto package was the established pattern. Key choices:

- STANDARD buf lint ruleset with 3 intentional inline ignores for `SERVICE_SUFFIX` and `Watch*` streaming response naming conventions
- AIP-158 pagination on list RPCs
- `FieldMask` on Telephony Update

### Decision

6 proto files covering all 26 RPCs; `revocall.engagement.v1` package; all enum values prefixed, zero values `_UNSPECIFIED`, reserved ranges in place; `buf lint` + `buf build` green.

## Implementation plan

### Design decisions locked in

- **`revocall.engagement.v1` package** — aligns with existing `revocall.*` proto namespace
- **STANDARD buf lint + 3 inline ignores** — `SERVICE_SUFFIX` (service name is `EngagementHub`, not `EngagementHubService`) and `Watch*` streaming response naming (deviates from STANDARD convention intentionally)
- **AIP-158 pagination** — `page_size` / `page_token` / `next_page_token` on all list RPCs
- **`FieldMask` on Telephony Update** — partial update semantics from the start
- **All enum zero values `_UNSPECIFIED`** — prevents accidental use of uninitialised fields; reserved ranges guard against proto-number reuse

### Tasks

1. `engagement.proto` — Engagement entity, 5 enums, reference types (`JourneyVersionRef`, `ContactRef`, `PhoneNumber`, `PrincipalRef`, `VoiceConfig`), `EngagementError`
2. `control.proto` — Start/Stop/Cancel/ResolveRoute/IssueVoiceTestToken request+response messages
3. `watch.proto` — WatchEngagement/Engagements requests, `EngagementEvent` with 12-type payload `oneof`, filter helpers
4. `query.proto` — Get/List engagements, List events (AIP-158 pagination), GetEngagementJourneyTimeline
5. `proxied.proto` — Transcript/Summary/Sentiment/OutputExtraction, call logs, Telephony CRUD, Analytics × 4 with typed enums
6. `service.proto` — service `EngagementHub` with 26 RPCs (`Watch*` server-streaming, rest unary)
7. buf lint + build validation — STANDARD ruleset, 3 inline ignores for `Watch*` naming and `SERVICE_SUFFIX`

### Deferred

None — proto package is self-contained; real prost/tonic codegen wired in T1-03+.
