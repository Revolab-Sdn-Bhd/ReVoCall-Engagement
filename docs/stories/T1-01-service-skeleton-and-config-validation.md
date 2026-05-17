# T1-01: Service skeleton + config validation

**Issue:** #8 | **Branch:** feat/8-service-skeleton | **Date:** 2026-05-15

## Brainstorm

### Problem

EH service needed a compilable skeleton before any business logic (T1-02+) could land: Cargo workspace, three servers (external gRPC :8443, internal gRPC :8444, HTTP :9090), Postgres schema, and prod-idle guard to satisfy PRD §7 B-new-3.

### Options considered

N/A — foundational scaffolding with a single obvious layout. Key design choices were:

- 4-crate workspace (`engagement-hub` binary + `-domain`, `-ports`, `-adapters` empty libs)
- Prod-idle guard checking `EH_ENV=production` + `EH_REGISTRY_ADAPTER=stub` + `!EH_TRACK_0_IDLE_MODE` → exit 78
- PRD §11 schema verbatim with two corrections (tightened `engagements_contact_check` to `IN (1,2)`, dropped redundant `engagement_events_event_pk_idx`)

### Decision

Single binary crate with three servers; adapter-kind metric ready for PRD §7 alert without relabeling; connection pool sized per PRD guidelines (10–25 conns, 5min idle, 5s statement_timeout, 3s acquire, 30min max lifetime); gRPC TCP+HTTP/2 keepalives every 30s; tonic-health flips to NOT_SERVING during drain.

## Implementation plan

### Design decisions locked in

- **4-crate workspace** — `engagement-hub` binary + `-domain`, `-ports`, `-adapters` empty libs; workspace deps pinned
- **Prod-idle guard** — all four PRD §7 B-new-3 combos enforced; exit 78 on violation; warning on `idle_mode+grpc` combo
- **Initial migration** — PRD §11 schema with two review-caught corrections: `engagements_contact_check` tightened to `IN (1,2)`; redundant `engagement_events_event_pk_idx` removed
- **Adapter-kind metric** — `engagementhub_registry_adapter_kind{kind,env,idle_mode}`; no relabel needed for PRD §7 alert
- **Connection pool** — 10–25 conns, 5min idle, 5s `statement_timeout`, 3s acquire, 30min max lifetime
- **gRPC config** — TCP+HTTP/2 keepalives every 30s; drain → NOT_SERVING via tonic-health

### Tasks

1. Cargo workspace setup — 4 crates, workspace deps pinned
2. Three-server scaffold — external gRPC :8443, internal :8444, HTTP :9090 with Prometheus + `/livez` + `/readyz`
3. Prod-idle guard — all four PRD §7 B-new-3 combos enforced; exit 78 on violation; warning on `idle_mode+grpc` combo
4. Initial migration — PRD §11 schema with two review-caught corrections (contact_check tightened, redundant index removed)
5. Adapter-kind metric — `engagementhub_registry_adapter_kind{kind,env,idle_mode}`; no relabel needed
6. Connection pool + gRPC config — pool params, keepalives, drain → NOT_SERVING
7. Tests — 15 tests (config combos, subprocess startup, advisory lock, gRPC health, HTTP, metrics); 2 added post-review

### Deferred

| Item | Reason |
| ------ | -------- |
| `pg_indexes` assertions in migration test | Deferred to avoid brittle schema coupling |
| Test isolation: wrap integration tests in `BEGIN`/`ROLLBACK` | Coordinate with T1 test infrastructure story |
| SIGTERM → exit-0 explicit test | Needs subprocess harness improvements |
| `engagement_audit_principal_time_idx` (add now vs T1-08) | Coordinate with reconciler story |
| Migrations-via-binary vs separate k8s Job | Coordinate with T7-01 |
| Prometheus `Content-Type` header on `/metrics` | Low priority; no consumer blocked |
| `wait_for_signal()` returns `io::Result<()>` instead of `.expect()` | Ergonomic improvement, not correctness |
| Narrow `pub` surface on `Shutdown` / `GrpcServers` | Refactor when consumers are known |
| `Env::Staging` test in `config_validation.rs` | Extend once staging env is exercised in CI |
