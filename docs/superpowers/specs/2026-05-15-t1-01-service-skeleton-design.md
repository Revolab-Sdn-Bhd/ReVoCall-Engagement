# T1-01: Service skeleton + config validation + initial DB schema — design

**Issue:** [#8](https://github.com/Revolab-Sdn-Bhd/ReVoCall-Engagement/issues/8)
**Track:** T1 — EH service core
**Source PRD:** `docs/2026-05-13-engagement-hub-prd.md` (companion docs repo) §§ 7, 11, 12.5
**Status:** Approved 2026-05-15

## Scope

Stand up the Rust + tonic binary, the workspace skeleton, the Postgres schema, and the startup-time config validation that every subsequent T1 story builds on. T1-01 ships **no business RPC services** — the gRPC servers register only the `grpc.health.v1` service. Real RPCs land in later stories (T1-06+, T1-10..T1-12).

## Repository layout

```
ReVoCall-Engagement/
├── Cargo.toml                                 # workspace manifest, resolver = "2"
├── rust-toolchain.toml                        # pinned stable, edition 2021
├── Dockerfile
├── docker-compose.yml                         # Postgres 16 for local + CI
├── justfile                                   # db-up, db-reset, migrate, test, run-dev
├── .github/workflows/ci.yml                   # fmt + clippy + test
├── crates/
│   ├── engagement-hub/                        # binary; only crate with real code in T1-01
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs                        # entrypoint + signal handling
│   │   │   ├── config.rs                      # env parsing + validation rule
│   │   │   ├── db.rs                          # PgPool builder + migration runner
│   │   │   ├── metrics.rs                     # Prometheus registry + adapter_kind gauge
│   │   │   └── server.rs                      # external + internal + http servers
│   │   └── tests/
│   │       ├── config_validation.rs           # 4 PRD config combos
│   │       ├── startup.rs                     # bind ports, idle-mode refusal, /metrics
│   │       └── advisory_lock.rs               # concurrent sequence allocation
│   ├── engagement-hub-domain/                 # empty lib stub (T1-02+ populates)
│   ├── engagement-hub-ports/                  # empty lib stub (T1-02 populates)
│   └── engagement-hub-adapters/               # empty lib stub (T1-03+ populates)
├── migrations/
│   ├── 20260515000000_initial_schema.up.sql   # full PRD §11 schema
│   └── 20260515000000_initial_schema.down.sql # drops in reverse
├── proto/                                     # placeholder for T2-01
└── clients/go/engagementhub/                  # placeholder for T2-03
```

The three sibling stub crates are created as empty `lib`s (single `lib.rs` with a `//` placeholder comment) so future stories add modules without restructuring the workspace.

## Configuration

### Environment variables

| Var | Required? | Default | Purpose |
|---|---|---|---|
| `EH_ENV` | yes | — | `dev` \| `staging` \| `production` |
| `EH_REGISTRY_ADAPTER` | yes | — | `stub` \| `grpc` |
| `EH_TRACK_0_IDLE_MODE` | no | `false` | When true, skip binding the external gRPC port |
| `EH_DATABASE_URL` | yes | — | Postgres DSN |
| `EH_EXTERNAL_GRPC_ADDR` | no | `0.0.0.0:8443` | External gRPC bind addr |
| `EH_INTERNAL_GRPC_ADDR` | no | `0.0.0.0:8444` | Internal gRPC bind addr |
| `EH_HTTP_ADDR` | no | `0.0.0.0:9090` | Prometheus + probes bind addr |
| `EH_DB_POOL_MIN` | no | `10` | sqlx min connections |
| `EH_DB_POOL_MAX` | no | `25` | sqlx max connections |
| `EH_DB_IDLE_TIMEOUT_SECS` | no | `300` | sqlx idle timeout |
| `EH_DB_STATEMENT_TIMEOUT_MS` | no | `5000` | per-session `SET statement_timeout` |
| `EH_DB_SLOW_QUERY_MS` | no | `500` | slow-query log threshold |
| `EH_LOG_FORMAT` | no | `json` | `json` \| `pretty` for tracing-subscriber |

Parsed via `clap` with `derive(Parser)` and `env` attributes so test code can construct `Config` directly without env shenanigans.

### Validation rule

The only rule with teeth in T1-01 (PRD §7 prod-idle guard, B-new-3):

```
if env == "production" && adapter == "stub" && !idle_mode {
    eprintln!("EH_REGISTRY_ADAPTER=stub is forbidden in production unless EH_TRACK_0_IDLE_MODE=true");
    std::process::exit(78);  // EX_CONFIG
}
```

All four combinations enumerated by the PRD:

| env | adapter | idle_mode | outcome |
|---|---|---|---|
| `production` | `stub` | `true` | start (idle) |
| `production` | `stub` | `false` | **exit 78** |
| `production` | `grpc` | any | start |
| `staging` \| `dev` | any | any | start |

## Startup sequence

```
1. parse env → Config (clap)
2. validate Config → exit 78 on InvalidConfig
3. init tracing-subscriber (JSON or pretty per EH_LOG_FORMAT)
4. build sqlx PgPool:
     PgPoolOptions::new()
       .min_connections(cfg.db_pool_min)
       .max_connections(cfg.db_pool_max)
       .idle_timeout(Duration::from_secs(cfg.db_idle_timeout_secs))
       .after_connect(|conn, _meta| async move {
           sqlx::query!("SET statement_timeout = $1; SET application_name = 'engagement-hub'",
                        cfg.db_statement_timeout_ms).execute(conn).await?;
           Ok(())
       })
5. run migrations: sqlx::migrate!("../../migrations").run(&pool).await
   on error → exit 70 (EX_SOFTWARE)
6. init Prometheus registry; set engagementhub_registry_adapter_kind{kind} = 1
7. spawn three servers concurrently:
     a. HTTP :9090 (axum)
          GET /metrics  → Prometheus text exposition
          GET /livez    → 200 OK always
          GET /readyz   → 200 OK unless draining; 503 SERVICE UNAVAILABLE when draining
     b. internal gRPC :8444 (tonic + tonic-health)
          grpc.health.v1.Health.Check → SERVING for all (no services registered yet)
     c. external gRPC :8443 (tonic + tonic-health)
          ONLY bound when !idle_mode
          identical health behavior
8. await SIGTERM | SIGINT
9. set draining=true (readyz returns 503)
10. shut down servers (no per-stream drain yet — T1-12)
11. drop pool, exit 0
```

Step 10 deliberately skips the per-Watch-stream drain (§12.5 lifecycle steps 2-7); those land in T1-12 when streams exist.

## Database migration

`migrations/20260515000000_initial_schema.up.sql` contains, in order:

1. `CREATE FUNCTION trg_set_updated_at()` — generic trigger fn used by `engagements`.
2. `CREATE TABLE engagements` + all indices + `engagements_set_updated_at` trigger.
3. `CREATE TABLE engagement_invocations` + index.
4. `CREATE TABLE route_resolutions` + index.
5. `CREATE TABLE engagement_events` + indices + `event_pk BIGSERIAL UNIQUE` column.
6. `CREATE FUNCTION trg_notify_engagement_event()` + `engagement_events_notify` trigger.
7. `CREATE TABLE engagement_audit` + indices.

The `.down.sql` drops in reverse: audit → events trigger + table + functions → resolutions → invocations → engagements + trigger + function.

The schema is copied **verbatim** from PRD §11 — no deviations. Status / channel / mode / contact_kind / created_by_kind / event_type / source / outcome remain `SMALLINT`; numeric enum values match the proto (locked in T2-01) but the migration only encodes the constraints that reference numeric values (e.g., `engagements_active_idx WHERE status IN (1, 2, 3)`).

Forward-only in prod; the `.down.sql` exists for local-dev rollback only. Documented in the migration's header comment.

## Metrics surface (T1-01)

Only one metric is required by the issue:

- `engagementhub_registry_adapter_kind` — `IntGaugeVec` with label `kind ∈ {"stub", "grpc"}`. On startup the active kind is set to `1`, the other to `0`.

The Prometheus registry is initialized in T1-01 so future stories (T1-02+) can register additional metrics without re-bootstrapping.

## gRPC health

Both gRPC servers register `tonic_health::server::health_reporter()`. No business services are registered, so the overall server health is `SERVING` as long as the process is up and not draining. k8s probes use `grpc_health_probe` against `:8443` (external) and `:8444` (internal); when idle mode is active, the external probe is omitted from the Deployment manifest (T7-01's concern).

## Testing

### Unit: `tests/config_validation.rs`

Four cases, asserted via `Config::validate()` (no env mutation; pass struct values directly):

| # | env | adapter | idle_mode | expect |
|---|---|---|---|---|
| 1 | `production` | `stub` | `true` | `Ok` |
| 2 | `production` | `stub` | `false` | `Err(InvalidConfig)` |
| 3 | `production` | `grpc` | `false` | `Ok` |
| 4 | `dev` | `stub` | `false` | `Ok` |

### Integration: `tests/startup.rs`

Requires a live Postgres (provided by `docker-compose` in dev, by the CI workflow's `postgres:16` service in CI).

- **Normal start (dev / stub / non-idle):** spin up server in a background task; poll `:9090/livez`, `:9090/readyz`, gRPC health on `:8443` and `:8444`; assert all SERVING. Scrape `:9090/metrics`; assert `engagementhub_registry_adapter_kind{kind="stub"} 1` present.
- **Idle mode:** spin up with `idle_mode=true`; assert `:8444` health SERVING, `:9090/metrics` reachable, **TCP connect to `:8443` fails** (port not bound). Use a short connect timeout and `connect().await` returning `Err`.
- **Invalid config:** call `Config { env: "production", adapter: "stub", idle_mode: false, ... }.validate()` → assert `Err(InvalidConfig)`. (Exit-78 behavior asserted via a separate `assert_cmd`-style subprocess test in unit suite.)

### Migration up/down: `tests/startup.rs` (same fixture)

After the startup test:
1. Connect a separate sqlx pool to the same DB.
2. `SELECT count(*) FROM information_schema.tables WHERE table_name IN ('engagements', 'engagement_invocations', 'route_resolutions', 'engagement_events', 'engagement_audit')` → `5`.
3. `\d+ engagements` equivalent via `pg_indexes` query → asserts each of the named indices in PRD §11 exists.
4. Execute the `.down.sql` contents, assert table count → 0. Re-run `migrate!()` → tables back. Clean.

### Advisory lock: `tests/advisory_lock.rs`

Validates the pattern that T1-02+ will rely on (per acceptance criteria). T1-01 ships no helper; the test embeds the SQL directly.

1. Insert one synthetic `engagements` row with a known `engagement_id`.
2. Spawn two `tokio::spawn`'d tasks; each opens its own connection, `BEGIN`, `SELECT pg_advisory_xact_lock(hashtext($1::text))`, then:
   ```sql
   INSERT INTO engagement_events (event_id, engagement_id, organization_id, sequence,
                                  event_type, status_after, source)
   VALUES (gen_random_uuid(), $1, $2,
           COALESCE((SELECT MAX(sequence) FROM engagement_events WHERE engagement_id = $1), 0) + 1,
           1, 1, 1)
   RETURNING sequence;
   COMMIT;
   ```
3. `join!` both. Assert sequences are `{1, 2}` (set equality — order is timing-dependent) with no duplicates.
4. Repeat the run 5× to surface flakes (each run uses a fresh engagement_id).

### CI

`.github/workflows/ci.yml`:

```yaml
services:
  postgres:
    image: postgres:16
    env:
      POSTGRES_PASSWORD: eh_test
      POSTGRES_DB: engagement_hub_db
    ports: ["5432:5432"]
    options: >-
      --health-cmd pg_isready --health-interval 5s
      --health-timeout 5s --health-retries 5

steps:
  - uses: actions/checkout@v4
  - uses: dtolnay/rust-toolchain@stable
  - run: cargo fmt --all -- --check
  - run: cargo clippy --workspace --all-targets -- -D warnings
  - run: cargo test --workspace
    env:
      EH_DATABASE_URL: postgres://postgres:eh_test@localhost:5432/engagement_hub_db
```

## Definition-of-Done mapping (from issue #8)

| DoD item | Where met |
|---|---|
| Service starts and binds gRPC :8443 | `server::run_external` |
| `/health` endpoint for liveness/readiness | gRPC health on :8443/:8444 + HTTP `/livez`,`/readyz` on :9090 |
| Startup validation; invalid config exits 78 | `config::Config::validate` + `main.rs` exit |
| Idle mode blocks external traffic on EH port | `server::run` skips external bind when `idle_mode` |
| Internal + metrics stay live in idle mode | internal `:8444` and HTTP `:9090` always bind |
| Metric `engagementhub_registry_adapter_kind{kind}` | `metrics::init` |
| Tables, indices, constraints, triggers | `migrations/20260515000000_initial_schema.up.sql` |
| Advisory lock pattern validated | `tests/advisory_lock.rs` |
| Connection pool 10–25, 5min idle, 5s stmt timeout, 500ms slow-query log | `db::build_pool` |
| `sqlx-cli` scaffolding | `justfile` targets + `Dockerfile` + `docker-compose.yml` + docs in README |
| 4 config-combo unit tests | `tests/config_validation.rs` |
| Startup integration test | `tests/startup.rs` |
| Up/down migration test | `tests/startup.rs` migration block |
| Postgres 16 compatibility | `docker-compose.yml` + CI both pin `postgres:16` |
| Rolling-deploy phase-1 test | PRD §11 "rolling-deploy migration test" — the initial migration is the first schema, so the phase-1 test reduces to: the migration applies cleanly on an empty DB while a (hypothetical) prior pod is still serving zero rows. Encoded as a CI assertion that the migration applies without exclusive locks on existing tables (no existing tables → trivially passes). |

## Out of scope (deferred to later T1 stories)

- Port traits, fakes, adapters (T1-02 → T1-04).
- Domain types + state machine (T1-02).
- `StartEngagement` orchestration, saga, reconciler (T1-06 → T1-09).
- Per-Watch-stream drain (T1-12).
- `EngagementHubInternal` service (T1-11).
- Proto files (T2-01).
- Go SDK (T2-03+).
- OTEL exporters + dashboard (T3).
- kustomize manifests + NetworkPolicy (T7).

## Open questions

None at design time.
