# ReVoCall Engagement

Engagement Hub (EH) — the runtime-plane service that orchestrates voice engagements across Voice Manager and Journey Manager. Owns the engagement entity and exposes the gRPC surface consumed by admin-backend and outbound services via the Go SDK in `clients/go/engagementhub`.

See `docs/2026-05-13-engagement-hub-prd.md` (companion docs repo) for the authoritative design.

## Layout (planned)

- `crates/engagement-hub/` — `tonic` server, config, startup
- `crates/engagement-hub-ports/` — port traits (Registry / VM / JM / PostCall / Analytics)
- `crates/engagement-hub-adapters/` — concrete adapters (incl. `RegistryStubAdapter`)
- `crates/engagement-hub-domain/` — domain types + state machine
- `migrations/` — `sqlx-cli` SQL migrations
- `proto/` — `revocall.engagement.v1` + `revocall.engagement.internal.v1`
- `clients/go/engagementhub/` — Go SDK

## Status

Track 0 — service core under construction. Not yet wired to any caller.
