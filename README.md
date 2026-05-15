# ReVoCall Engagement Hub

Engagement Hub (EH) — the runtime-plane service that orchestrates voice engagements across Voice Manager and Journey Manager. Owns the engagement entity and (eventually) the gRPC surface consumed by admin-backend and outbound services.

T1-01 ships the workspace scaffold, config validation, initial DB schema, and gRPC + HTTP plumbing. No business RPCs yet — those land in later T1 stories.

## Layout

- `crates/engagement-hub/` — `tonic` server, config, startup, integration tests
- `crates/engagement-hub-ports/` — port traits (T1-02 populates)
- `crates/engagement-hub-adapters/` — concrete adapters (T1-03+ populates)
- `crates/engagement-hub-domain/` — domain types (T1-02+ populates)
- `migrations/` — `sqlx-cli` SQL migrations
- `proto/` — `revocall.engagement.v1` (T2-01 populates)
- `clients/go/engagementhub/` — Go SDK (T2-03+ populates)

## Quick start (local dev)

Prereqs: `cargo`, `docker`, `just`, `sqlx-cli` (`cargo install sqlx-cli --no-default-features --features postgres,rustls`).

```sh
just db-up       # Postgres 16 on :5432
just migrate     # apply schema
just run-dev     # boots EH with EH_ENV=dev EH_REGISTRY_ADAPTER=stub
```

Verify:

```sh
curl localhost:9090/livez                                          # ok
curl localhost:9090/metrics | grep engagementhub_registry_adapter  # kind="stub" 1
grpc_health_probe -addr=localhost:8443                             # SERVING
grpc_health_probe -addr=localhost:8444                             # SERVING
```

## Environment variables

| Var | Required? | Default | Purpose |
|---|---|---|---|
| `EH_ENV` | yes | — | `dev` \| `staging` \| `production` |
| `EH_REGISTRY_ADAPTER` | yes | — | `stub` \| `grpc` (only `stub` wired in T1-01) |
| `EH_TRACK_0_IDLE_MODE` | no | `false` | When true, skip binding external gRPC `:8443` |
| `EH_DATABASE_URL` | yes | — | Postgres DSN |
| `EH_EXTERNAL_GRPC_ADDR` | no | `0.0.0.0:8443` | External gRPC bind |
| `EH_INTERNAL_GRPC_ADDR` | no | `0.0.0.0:8444` | Internal gRPC bind |
| `EH_HTTP_ADDR` | no | `0.0.0.0:9090` | HTTP `/metrics`, `/livez`, `/readyz` |
| `EH_DB_POOL_MIN` | no | `10` | sqlx min conns |
| `EH_DB_POOL_MAX` | no | `25` | sqlx max conns |
| `EH_DB_IDLE_TIMEOUT_SECS` | no | `300` | sqlx idle timeout |
| `EH_DB_STATEMENT_TIMEOUT_MS` | no | `5000` | per-session `SET statement_timeout` |
| `EH_DB_SLOW_QUERY_MS` | no | `500` | slow-query log threshold |
| `EH_DB_ACQUIRE_TIMEOUT_SECS` | no | `3` | Max seconds to wait for a pool connection |
| `EH_DB_MAX_LIFETIME_SECS` | no | `1800` | Max lifetime (s) of a pooled connection (30 min) |
| `EH_LOG_FORMAT` | no | `json` | `json` \| `pretty` |

## Prod-idle guard

`EH_ENV=production` + `EH_REGISTRY_ADAPTER=stub` requires `EH_TRACK_0_IDLE_MODE=true`. Any other combination of those three rejects with exit code 78. See PRD §7.

## Tests

```sh
just test
# or
cargo test --workspace -- --test-threads=1
```

`--test-threads=1` is required: the migration up/down test wipes tables and races otherwise.
