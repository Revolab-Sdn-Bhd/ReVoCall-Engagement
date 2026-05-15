# T3-02: EH-specific metric set instrumentation

**Issue:** #33 | **Branch:** feat/33-eh-metric-set-instrumentation | **Date:** 2026-05-15

## Brainstorm

### Problem

T3-01 established the OTEL trace pipeline. The `Metrics` struct currently holds only 2 metrics (`registry_adapter_kind`, `otel_exporter_dropped_spans_total`). T3-03 (dashboard) and T3-04 (alert rules) both depend on the full 28-metric surface being registered, named consistently, and scraped by Prometheus before their work can begin. Without a consolidation step, individual T1 stories would drift on label names and bucket boundaries, causing silent panel failures in the dashboard and incorrect alert thresholds.

### Options considered

**Struct organisation — flat vs. grouped:**
- *Grouped sub-structs by domain* (`RpcMetrics`, `LifecycleMetrics`, etc.): cleaner files but two-level call sites (`metrics.lifecycle.engagements_started`) and forces T1 story authors to know which domain group their metric lives in.
- *Flat submodules with flat fields*: organised files, same flat call-site ergonomics as flat struct, but adds indirection for no real payoff at 28 metrics.
- *Flat struct (chosen)*: all 28 fields on `Metrics`, `new()` grows to ~150 lines but is consistent with existing code. T1 call sites are `metrics.rpc_total.with_label_values(&[...]).inc()` — simplest possible.

**Metrics export mechanism:**
The acceptance criteria says "via OTLP alongside traces." Infrastructure audit of `staging-001/infrastructure/grafana-alloy/config.hcl` shows two completely separate pipelines: OTLP/gRPC → Tempo for traces, and `prometheus.scrape` → Mimir for metrics. No service in the cluster uses the OTLP metrics API. "Via OTLP" in the PRD acceptance criteria is a documentation error — the correct path is Prometheus pull-scrape → Grafana Alloy → Grafana Cloud, which is what the existing `/metrics` endpoint on `:9090` already serves.

**`listen_notify_consumer_lag_events` classification:**
PRD §10.4 lists it under Histograms; PRD prose (line 1934) calls it "gauge per replica." The acceptance criteria says 9 histograms + 6 gauges (6 explicitly named, not including this one). Deferred to T1-09 (LISTEN/NOTIFY fanout story), which owns the emission call site and can resolve the classification at that time.

### Decision

Flat struct (Approach A). All 28 metrics added to the existing `Metrics` struct in `metrics.rs`. Prometheus scrape path — no OTLP metrics. `listen_notify_consumer_lag_events` deferred to T1-09.

T3-02 owns:
1. The canonical `Metrics` struct with all 28 metrics registered
2. Histogram bucket boundaries (documented below)
3. Pre-initialization of static label sets so all series appear at zero on first scrape
4. Five unit tests (counters registered, histograms registered, gauges registered, static series at zero, no `organization_id` label)
5. Kustomize PR: add `engagement-hub` namespace to Alloy scrape list + named `metrics` port

T3-02 does NOT own call sites (T1 stories) or the 100-engagement load test (T3-03 DoD).

## Implementation plan

### Design decisions

**Histogram bucket boundaries (all in seconds):**

| Metric | Buckets |
|---|---|
| `startup_duration_seconds{stage}` | `[0.001, 0.010, 0.050, 0.100, 0.500, 1.0, 5.0]` |
| `rpc_duration_seconds{rpc, code}` | `[0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5, 5.0]` |
| `adapter_duration_seconds{target, method, code}` | `[0.001, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5]` |
| `orchestration_duration_seconds{stage}` | `[0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5, 5.0]` |
| `audit_insert_duration_seconds` | `[0.001, 0.002, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500]` |
| `listen_notify_fanout_latency_seconds` | `[0.001, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5]` |
| `time_to_first_response_seconds` | `[0.100, 0.250, 0.500, 1.0, 2.0, 5.0, 10.0, 30.0]` |
| `watch_stream_duration_seconds` | `[10.0, 60.0, 300.0, 1800.0, 3600.0, 14400.0, 86400.0]` |
| `call_duration_seconds{outcome}` | `[5.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0]` |

**Pre-initialized static label sets:**

| Metric | Pre-seeded values |
|---|---|
| `reconciler_backlog{class}` | `pending_engagement`, `orphan_compensation`, `pending_audit`, `overrun_live` |
| `startup_duration_seconds{stage}` | `validate_and_commit`, `registry_resolve`, `route_resolved_commit`, `parallel_bind`, `invocation_requested_commit`, `audit` |
| `orchestration_duration_seconds{stage}` | `start_engagement`, `stop_engagement`, `cancel_engagement`, `saga_compensation` |
| `otel_exporter_dropped_spans_total{exporter}` | `grafana`, `langfuse`, `local` (already done — regression guard only) |

**Prometheus type choices:**
- Zero-label counters → `IntCounter`
- Labeled counters → `IntCounterVec`
- Zero-label histograms → `Histogram`
- Labeled histograms → `HistogramVec`
- Zero-label gauges → `IntGauge`
- Labeled gauges → `IntGaugeVec`

### Tasks

_To be filled in by writing-plans._

### Deferred

- `listen_notify_consumer_lag_events` — classification (histogram vs gauge) and emission owned by T1-09
- `metrics.rpc_total.inc(...)` call sites in query/control RPC handlers — T1-10, T1-11, T1-12 (comments left on those issues)
- `active_engagements`, `in_flight_invocations` gauge set/dec in lifecycle transitions — T1-06, T1-11
- `db_pool_in_use / idle` wired to sqlx pool telemetry — T1-05 or T1-06
- 100-engagement load test with real values — T3-03 DoD
- Dashboard JSON — T3-03
- Alert rules + recording rules — T3-04
