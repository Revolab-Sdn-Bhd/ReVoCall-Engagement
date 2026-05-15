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

> **For agentic workers:** use `superpowers:subagent-driven-development` + `superpowers:test-driven-development` to implement task-by-task.

---

#### Task 1 — Write all 5 failing tests (TDD red phase)

**Files:**
- Modify: `crates/engagement-hub/src/metrics.rs` (replace existing `mod tests` block)

- [ ] **Step 1.1 — Replace the `mod tests` block** with the full 5-test suite below. The two existing tests (`active_kind_has_all_three_labels`, `dropped_spans_counter_registered_with_all_labels`) are preserved inside the block.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_counters_registered() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        let text = m.gather_text().unwrap();
        for name in [
            "engagementhub_rpc_total",
            "engagementhub_engagements_started_total",
            "engagementhub_engagements_terminal_total",
            "engagementhub_engagement_errors_total",
            "engagementhub_watch_streams_opened_total",
            "engagementhub_watch_stream_reconnects_total",
            "engagementhub_reconciler_swept_total",
            "engagementhub_adapter_retries_total",
            "engagementhub_saga_compensation_outcome_total",
            "engagementhub_audit_insert_failures_total",
            "engagementhub_listen_notify_reconnects_total",
            "engagementhub_otel_exporter_dropped_spans_total",
            "engagementhub_db_failover_detected_total",
        ] {
            assert!(text.contains(name), "counter missing: {name}\n\nfull output:\n{text}");
        }
    }

    #[test]
    fn all_histograms_registered() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        let text = m.gather_text().unwrap();
        for name in [
            "engagementhub_rpc_duration_seconds",
            "engagementhub_adapter_duration_seconds",
            "engagementhub_orchestration_duration_seconds",
            "engagementhub_startup_duration_seconds",
            "engagementhub_call_duration_seconds",
            "engagementhub_time_to_first_response_seconds",
            "engagementhub_watch_stream_duration_seconds",
            "engagementhub_listen_notify_fanout_latency_seconds",
            "engagementhub_audit_insert_duration_seconds",
        ] {
            assert!(
                text.contains(&format!("{name}_bucket"))
                    || text.contains(&format!("{name}_sum")),
                "histogram missing: {name}\n\nfull output:\n{text}"
            );
        }
    }

    #[test]
    fn all_gauges_registered() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        let text = m.gather_text().unwrap();
        for name in [
            "engagementhub_active_engagements",
            "engagementhub_active_watches",
            "engagementhub_in_flight_invocations",
            "engagementhub_db_pool_in_use",
            "engagementhub_db_pool_idle",
            "engagementhub_reconciler_backlog",
            "engagementhub_registry_adapter_kind",
        ] {
            assert!(text.contains(name), "gauge missing: {name}\n\nfull output:\n{text}");
        }
    }

    #[test]
    fn pre_initialized_series_appear_at_zero() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        let text = m.gather_text().unwrap();
        // reconciler_backlog — all 4 class values
        for class in [
            "pending_engagement",
            "orphan_compensation",
            "pending_audit",
            "overrun_live",
        ] {
            assert!(
                text.contains(&format!(
                    "engagementhub_reconciler_backlog{{class=\"{class}\"}} 0"
                )),
                "reconciler_backlog class={class} missing at zero\n\nfull output:\n{text}"
            );
        }
        // startup_duration_seconds — all 6 stage values
        for stage in [
            "validate_and_commit",
            "registry_resolve",
            "route_resolved_commit",
            "parallel_bind",
            "invocation_requested_commit",
            "audit",
        ] {
            assert!(
                text.contains(&format!(
                    "engagementhub_startup_duration_seconds_count{{stage=\"{stage}\"}} 0"
                )),
                "startup_duration stage={stage} missing\n\nfull output:\n{text}"
            );
        }
        // orchestration_duration_seconds — all 4 stage values
        for stage in [
            "start_engagement",
            "stop_engagement",
            "cancel_engagement",
            "saga_compensation",
        ] {
            assert!(
                text.contains(&format!(
                    "engagementhub_orchestration_duration_seconds_count{{stage=\"{stage}\"}} 0"
                )),
                "orchestration_duration stage={stage} missing\n\nfull output:\n{text}"
            );
        }
        // otel_exporter_dropped_spans_total — regression guard
        for exporter in ["grafana", "langfuse", "local"] {
            assert!(
                text.contains(&format!("exporter=\"{exporter}\"")),
                "dropped_spans exporter={exporter} missing\n\nfull output:\n{text}"
            );
        }
    }

    #[test]
    fn no_organization_id_label() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        let text = m.gather_text().unwrap();
        assert!(
            !text.contains("organization_id"),
            "organization_id found as Prometheus label — high-cardinality footgun!\n\nfull output:\n{text}"
        );
    }

    // --- existing tests kept below ---

    #[test]
    fn active_kind_has_all_three_labels() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        let text = m.gather_text().unwrap();
        assert!(
            text.contains(
                r#"engagementhub_registry_adapter_kind{env="dev",idle_mode="false",kind="stub"} 1"#
            ),
            "missing active=stub line with env+idle_mode in:\n{text}"
        );
    }

    #[test]
    fn dropped_spans_counter_registered_with_all_labels() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        let text = m.gather_text().unwrap();
        assert!(
            text.contains("engagementhub_otel_exporter_dropped_spans_total"),
            "counter missing:\n{text}"
        );
        for exporter in ["grafana", "langfuse", "local"] {
            assert!(
                text.contains(&format!(r#"exporter="{exporter}""#)),
                "label exporter={exporter} missing:\n{text}"
            );
        }
    }
}
```

- [ ] **Step 1.2 — Run the tests**

```bash
cargo test -p engagement-hub 2>&1 | grep -E "test .* (ok|FAILED|ignored)"
```

Expected: `active_kind_has_all_three_labels ok`, `dropped_spans_counter_registered_with_all_labels ok`, `no_organization_id_label ok` (passes immediately — existing metrics have no `organization_id`). The 4 feature tests (`all_counters_registered`, `all_histograms_registered`, `all_gauges_registered`, `pre_initialized_series_appear_at_zero`) all `FAILED` because the new metric names are not in `gather_text()` output yet.

- [ ] **Step 1.3 — Commit**

```bash
git add crates/engagement-hub/src/metrics.rs
git commit -m "test(t3-02): write 5 failing metric-surface tests"
```

---

#### Task 2 — Add 12 new counter fields (green: `all_counters_registered`)

**Files:**
- Modify: `crates/engagement-hub/src/metrics.rs`

- [ ] **Step 2.1 — Update the `use` line** at the top of `metrics.rs`:

```rust
use prometheus::{IntCounter, IntCounterVec, IntGaugeVec, Opts, Registry};
```

- [ ] **Step 2.2 — Add 12 new counter fields to the `Metrics` struct** after the existing `otel_exporter_dropped_spans` field:

```rust
pub struct Metrics {
    pub registry: Registry,
    pub registry_adapter_kind: IntGaugeVec,
    pub otel_exporter_dropped_spans: IntCounterVec,
    // --- Counters (new) ---
    pub rpc_total: IntCounterVec,
    pub engagements_started_total: IntCounterVec,
    pub engagements_terminal_total: IntCounterVec,
    pub engagement_errors_total: IntCounterVec,
    pub watch_streams_opened_total: IntCounter,
    pub watch_stream_reconnects_total: IntCounter,
    pub reconciler_swept_total: IntCounterVec,
    pub adapter_retries_total: IntCounterVec,
    pub saga_compensation_outcome_total: IntCounterVec,
    pub audit_insert_failures_total: IntCounter,
    pub listen_notify_reconnects_total: IntCounter,
    pub db_failover_detected_total: IntCounter,
}
```

- [ ] **Step 2.3 — Add registration code inside `Metrics::new()`**, after the `otel_exporter_dropped_spans` block and before `Ok(Self { ... })`:

```rust
        let rpc_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_rpc_total",
                "Total RPC calls by method, status code, and caller",
            ),
            &["rpc", "code", "caller_service"],
        )?;
        registry.register(Box::new(rpc_total.clone()))?;

        let engagements_started_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_engagements_started_total",
                "Total engagements started by channel and mode",
            ),
            &["channel", "mode"],
        )?;
        registry.register(Box::new(engagements_started_total.clone()))?;

        let engagements_terminal_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_engagements_terminal_total",
                "Total engagements reaching a terminal status",
            ),
            &["terminal_status"],
        )?;
        registry.register(Box::new(engagements_terminal_total.clone()))?;

        let engagement_errors_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_engagement_errors_total",
                "Total engagement errors by error code",
            ),
            &["error_code"],
        )?;
        registry.register(Box::new(engagement_errors_total.clone()))?;

        let watch_streams_opened_total = IntCounter::new(
            "engagementhub_watch_streams_opened_total",
            "Total watch streams opened",
        )?;
        registry.register(Box::new(watch_streams_opened_total.clone()))?;

        let watch_stream_reconnects_total = IntCounter::new(
            "engagementhub_watch_stream_reconnects_total",
            "Total watch stream reconnection attempts",
        )?;
        registry.register(Box::new(watch_stream_reconnects_total.clone()))?;

        let reconciler_swept_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_reconciler_swept_total",
                "Total engagements swept by the reconciler, by source status and action taken",
            ),
            &["from_status", "action"],
        )?;
        registry.register(Box::new(reconciler_swept_total.clone()))?;

        let adapter_retries_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_adapter_retries_total",
                "Total adapter call retries by target, method, and attempt number",
            ),
            &["target", "method", "attempt_number"],
        )?;
        registry.register(Box::new(adapter_retries_total.clone()))?;

        let saga_compensation_outcome_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_saga_compensation_outcome_total",
                "Saga compensation outcomes by stage and result",
            ),
            &["stage", "result"],
        )?;
        registry.register(Box::new(saga_compensation_outcome_total.clone()))?;

        let audit_insert_failures_total = IntCounter::new(
            "engagementhub_audit_insert_failures_total",
            "Total audit insert failures — any non-zero rate is a compliance breach",
        )?;
        registry.register(Box::new(audit_insert_failures_total.clone()))?;

        let listen_notify_reconnects_total = IntCounter::new(
            "engagementhub_listen_notify_reconnects_total",
            "Total LISTEN/NOTIFY connection reconnections",
        )?;
        registry.register(Box::new(listen_notify_reconnects_total.clone()))?;

        let db_failover_detected_total = IntCounter::new(
            "engagementhub_db_failover_detected_total",
            "Total detected database failover events",
        )?;
        registry.register(Box::new(db_failover_detected_total.clone()))?;
```

- [ ] **Step 2.4 — Add all 12 new fields to `Ok(Self { ... })`**:

```rust
        Ok(Self {
            registry,
            registry_adapter_kind,
            otel_exporter_dropped_spans,
            rpc_total,
            engagements_started_total,
            engagements_terminal_total,
            engagement_errors_total,
            watch_streams_opened_total,
            watch_stream_reconnects_total,
            reconciler_swept_total,
            adapter_retries_total,
            saga_compensation_outcome_total,
            audit_insert_failures_total,
            listen_notify_reconnects_total,
            db_failover_detected_total,
        })
```

- [ ] **Step 2.5 — Run the counter test**

```bash
cargo test -p engagement-hub metrics::tests::all_counters_registered 2>&1
```

Expected: `test metrics::tests::all_counters_registered ... ok`

- [ ] **Step 2.6 — Commit**

```bash
git add crates/engagement-hub/src/metrics.rs
git commit -m "feat(t3-02): register all 13 counters"
```

---

#### Task 3 — Add 9 histogram fields (green: `all_histograms_registered`)

**Files:**
- Modify: `crates/engagement-hub/src/metrics.rs`

- [ ] **Step 3.1 — Update the `use` line** to add histogram types:

```rust
use prometheus::{Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, Opts, Registry};
```

- [ ] **Step 3.2 — Add 9 histogram fields to the `Metrics` struct** after `db_failover_detected_total`:

```rust
    // --- Histograms ---
    pub rpc_duration_seconds: HistogramVec,
    pub adapter_duration_seconds: HistogramVec,
    pub orchestration_duration_seconds: HistogramVec,
    pub startup_duration_seconds: HistogramVec,
    pub call_duration_seconds: HistogramVec,
    pub time_to_first_response_seconds: Histogram,
    pub watch_stream_duration_seconds: Histogram,
    pub listen_notify_fanout_latency_seconds: Histogram,
    pub audit_insert_duration_seconds: Histogram,
```

- [ ] **Step 3.3 — Add histogram registration + pre-init code inside `new()`**, after the counter block:

```rust
        // --- Histograms ---
        let rpc_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "engagementhub_rpc_duration_seconds",
                "RPC handler duration in seconds",
            )
            .buckets(vec![0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5, 5.0]),
            &["rpc", "code"],
        )?;
        registry.register(Box::new(rpc_duration_seconds.clone()))?;

        let adapter_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "engagementhub_adapter_duration_seconds",
                "Adapter call duration in seconds",
            )
            .buckets(vec![0.001, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5]),
            &["target", "method", "code"],
        )?;
        registry.register(Box::new(adapter_duration_seconds.clone()))?;

        let orchestration_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "engagementhub_orchestration_duration_seconds",
                "Top-level orchestration operation duration in seconds",
            )
            .buckets(vec![0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5, 5.0]),
            &["stage"],
        )?;
        registry.register(Box::new(orchestration_duration_seconds.clone()))?;
        for stage in [
            "start_engagement",
            "stop_engagement",
            "cancel_engagement",
            "saga_compensation",
        ] {
            orchestration_duration_seconds.with_label_values(&[stage]);
        }

        let startup_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "engagementhub_startup_duration_seconds",
                "StartEngagement per-stage duration in seconds",
            )
            .buckets(vec![0.001, 0.010, 0.050, 0.100, 0.500, 1.0, 5.0]),
            &["stage"],
        )?;
        registry.register(Box::new(startup_duration_seconds.clone()))?;
        for stage in [
            "validate_and_commit",
            "registry_resolve",
            "route_resolved_commit",
            "parallel_bind",
            "invocation_requested_commit",
            "audit",
        ] {
            startup_duration_seconds.with_label_values(&[stage]);
        }

        let call_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "engagementhub_call_duration_seconds",
                "Total call duration in seconds by outcome",
            )
            .buckets(vec![
                5.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0,
            ]),
            &["outcome"],
        )?;
        registry.register(Box::new(call_duration_seconds.clone()))?;

        let time_to_first_response_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "engagementhub_time_to_first_response_seconds",
                "Time from engagement start to first AI response in seconds",
            )
            .buckets(vec![0.100, 0.250, 0.500, 1.0, 2.0, 5.0, 10.0, 30.0]),
        )?;
        registry.register(Box::new(time_to_first_response_seconds.clone()))?;

        let watch_stream_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "engagementhub_watch_stream_duration_seconds",
                "Watch stream session duration in seconds",
            )
            .buckets(vec![10.0, 60.0, 300.0, 1800.0, 3600.0, 14400.0, 86400.0]),
        )?;
        registry.register(Box::new(watch_stream_duration_seconds.clone()))?;

        let listen_notify_fanout_latency_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "engagementhub_listen_notify_fanout_latency_seconds",
                "LISTEN/NOTIFY fanout latency in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500, 1.0, 2.5,
            ]),
        )?;
        registry.register(Box::new(listen_notify_fanout_latency_seconds.clone()))?;

        let audit_insert_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "engagementhub_audit_insert_duration_seconds",
                "Audit row insert duration in seconds",
            )
            .buckets(vec![
                0.001, 0.002, 0.005, 0.010, 0.025, 0.050, 0.100, 0.250, 0.500,
            ]),
        )?;
        registry.register(Box::new(audit_insert_duration_seconds.clone()))?;
```

- [ ] **Step 3.4 — Add histogram fields to `Ok(Self { ... })`**:

```rust
        Ok(Self {
            registry,
            registry_adapter_kind,
            otel_exporter_dropped_spans,
            rpc_total,
            engagements_started_total,
            engagements_terminal_total,
            engagement_errors_total,
            watch_streams_opened_total,
            watch_stream_reconnects_total,
            reconciler_swept_total,
            adapter_retries_total,
            saga_compensation_outcome_total,
            audit_insert_failures_total,
            listen_notify_reconnects_total,
            db_failover_detected_total,
            rpc_duration_seconds,
            adapter_duration_seconds,
            orchestration_duration_seconds,
            startup_duration_seconds,
            call_duration_seconds,
            time_to_first_response_seconds,
            watch_stream_duration_seconds,
            listen_notify_fanout_latency_seconds,
            audit_insert_duration_seconds,
        })
```

- [ ] **Step 3.5 — Run histogram + counter tests**

```bash
cargo test -p engagement-hub metrics::tests 2>&1 | grep -E "test .* (ok|FAILED)"
```

Expected: `all_counters_registered ok`, `all_histograms_registered ok`. Pre-init, gauges, and no-org-id tests still `FAILED`.

- [ ] **Step 3.6 — Commit**

```bash
git add crates/engagement-hub/src/metrics.rs
git commit -m "feat(t3-02): register all 9 histograms with bucket boundaries and stage pre-init"
```

---

#### Task 4 — Add 6 gauge fields + pre-init (all 5 tests green)

**Files:**
- Modify: `crates/engagement-hub/src/metrics.rs`

- [ ] **Step 4.1 — Update the `use` line** to add gauge types:

```rust
use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec,
    Opts, Registry,
};
```

- [ ] **Step 4.2 — Add 6 gauge fields to the `Metrics` struct** after `audit_insert_duration_seconds`:

```rust
    // --- Gauges ---
    pub active_engagements: IntGaugeVec,
    pub active_watches: IntGaugeVec,
    pub in_flight_invocations: IntGauge,
    pub db_pool_in_use: IntGauge,
    pub db_pool_idle: IntGauge,
    pub reconciler_backlog: IntGaugeVec,
```

- [ ] **Step 4.3 — Add gauge registration + pre-init inside `new()`**, after the histogram block:

```rust
        // --- Gauges ---
        let active_engagements = IntGaugeVec::new(
            Opts::new(
                "engagementhub_active_engagements",
                "Current active engagements by status",
            ),
            &["status"],
        )?;
        registry.register(Box::new(active_engagements.clone()))?;

        let active_watches = IntGaugeVec::new(
            Opts::new(
                "engagementhub_active_watches",
                "Current active watch streams by filter type",
            ),
            &["filter_type"],
        )?;
        registry.register(Box::new(active_watches.clone()))?;

        let in_flight_invocations = IntGauge::new(
            "engagementhub_in_flight_invocations",
            "Current in-flight invocation requests",
        )?;
        registry.register(Box::new(in_flight_invocations.clone()))?;

        let db_pool_in_use = IntGauge::new(
            "engagementhub_db_pool_in_use",
            "Current database pool connections in use",
        )?;
        registry.register(Box::new(db_pool_in_use.clone()))?;

        let db_pool_idle = IntGauge::new(
            "engagementhub_db_pool_idle",
            "Current idle database pool connections",
        )?;
        registry.register(Box::new(db_pool_idle.clone()))?;

        let reconciler_backlog = IntGaugeVec::new(
            Opts::new(
                "engagementhub_reconciler_backlog",
                "Current reconciler backlog size by class",
            ),
            &["class"],
        )?;
        registry.register(Box::new(reconciler_backlog.clone()))?;
        // Pre-initialize all 4 static class values so they appear at zero on first scrape.
        for class in [
            "pending_engagement",
            "orphan_compensation",
            "pending_audit",
            "overrun_live",
        ] {
            reconciler_backlog.with_label_values(&[class]).set(0);
        }
```

- [ ] **Step 4.4 — Add gauge fields to `Ok(Self { ... })`** (final complete constructor return):

```rust
        Ok(Self {
            registry,
            registry_adapter_kind,
            otel_exporter_dropped_spans,
            rpc_total,
            engagements_started_total,
            engagements_terminal_total,
            engagement_errors_total,
            watch_streams_opened_total,
            watch_stream_reconnects_total,
            reconciler_swept_total,
            adapter_retries_total,
            saga_compensation_outcome_total,
            audit_insert_failures_total,
            listen_notify_reconnects_total,
            db_failover_detected_total,
            rpc_duration_seconds,
            adapter_duration_seconds,
            orchestration_duration_seconds,
            startup_duration_seconds,
            call_duration_seconds,
            time_to_first_response_seconds,
            watch_stream_duration_seconds,
            listen_notify_fanout_latency_seconds,
            audit_insert_duration_seconds,
            active_engagements,
            active_watches,
            in_flight_invocations,
            db_pool_in_use,
            db_pool_idle,
            reconciler_backlog,
        })
```

- [ ] **Step 4.5 — Run all tests**

```bash
cargo test -p engagement-hub 2>&1 | grep -E "test .* (ok|FAILED)"
```

Expected: all 7 tests `ok`. Zero failures.

- [ ] **Step 4.6 — Commit**

```bash
git add crates/engagement-hub/src/metrics.rs
git commit -m "feat(t3-02): register all 6 gauges and pre-initialize static series"
```

---

#### Task 5 — Full test run + smoke build + story doc update

**Files:**
- Modify: `docs/stories/T3-02-eh-metric-set-instrumentation.md`

- [ ] **Step 5.1 — Run the full test suite to confirm no regressions**

```bash
cargo test -p engagement-hub 2>&1 | tail -5
```

Expected output ends with: `test result: ok. N passed; 0 failed; 0 ignored`

- [ ] **Step 5.2 — Build the binary**

```bash
cargo build -p engagement-hub 2>&1 | tail -3
```

Expected: `Finished dev [unoptimized + debuginfo] target(s)` with no errors.

- [ ] **Step 5.3 — Update the story doc** to replace "To be filled in by writing-plans" Tasks placeholder with a reference to the actual tasks completed. The `## Implementation plan → ### Tasks` section should now contain: `_Tasks executed per plan in feat/33-eh-metric-set-instrumentation story doc. See git log for individual commits._`

- [ ] **Step 5.4 — Commit story doc**

```bash
git add docs/stories/T3-02-eh-metric-set-instrumentation.md
git commit -m "docs: add implementation plan to story doc #33"
```

---

#### Task 6 — Kustomize PR (separate PR to `RevoCall-Kustomize`)

**Files (in `RevoCall-Kustomize` repo):**
- Modify: `staging-001/infrastructure/grafana-alloy/config.hcl`

- [ ] **Step 6.1 — Change directory to the Kustomize repo**

```bash
cd /Users/chunzhe/Projects/RevoCall-Kustomize
git checkout main && git pull
git checkout -b feat/eh-alloy-scrape-namespace
```

- [ ] **Step 6.2 — Add `"engagement-hub"` to the Alloy namespace list** in `staging-001/infrastructure/grafana-alloy/config.hcl`. The `discovery.kubernetes "application_pods"` block currently reads:

```hcl
    namespaces {
        names = ["admin", "ai-handler", "backoffice", "chat", "outbound", "revcaf"]
    }
```

Change it to:

```hcl
    namespaces {
        names = ["admin", "ai-handler", "backoffice", "chat", "engagement-hub", "outbound", "revcaf"]
    }
```

- [ ] **Step 6.3 — Commit and open PR**

```bash
git add staging-001/infrastructure/grafana-alloy/config.hcl
git commit -m "feat: add engagement-hub to Alloy Prometheus scrape namespaces"
gh pr create \
  --title "feat: add engagement-hub to Alloy Prometheus scrape namespaces" \
  --body "$(cat <<'EOF'
## Summary
- Adds `engagement-hub` to the namespace list in `grafana-alloy/config.hcl` so Alloy's pod-discovery scrape rule picks up EH pods once T7-01 ships the Deployment.
- Alloy keeps only pods with `containerPort.name = metrics` — T7-01 must include `name: metrics` on containerPort 9090 in the Deployment spec for scrape to activate.

## Dependency
- T7-01 (#XX) must name the container port `metrics` at 9090 for scrape to activate.
- Can merge independently; no harm done before T7-01 ships (namespace has no pods yet).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 6.4 — Leave a comment on T7-01 GitHub issue** to communicate the port-name requirement:

```bash
cd /Users/chunzhe/Projects/ReVoCall-Engagement
# Find T7-01 issue number
gh issue list --repo Revolab-Sdn-Bhd/ReVoCall-Engagement --label "track:T7" --state open
# Then comment (replace <T7-01-ISSUE-NUMBER> with actual number):
gh issue comment 62 \
  --repo Revolab-Sdn-Bhd/ReVoCall-Engagement \
  --body "**From T3-02:** The Deployment spec must include \`name: metrics\` on containerPort 9090 for Grafana Alloy's pod-discovery scrape rule to pick up EH pods. The Alloy config update (adding \`engagement-hub\` to the namespace list) is being merged via a separate Kustomize PR as part of T3-02."
```

### Deferred

- `listen_notify_consumer_lag_events` — classification (histogram vs gauge) and emission owned by T1-09
- `metrics.rpc_total.inc(...)` call sites in query/control RPC handlers — T1-10, T1-11, T1-12 (comments left on those issues)
- `active_engagements`, `in_flight_invocations` gauge set/dec in lifecycle transitions — T1-06, T1-11
- `db_pool_in_use / idle` wired to sqlx pool telemetry — T1-05 or T1-06
- 100-engagement load test with real values — T3-03 DoD
- Dashboard JSON — T3-03
- Alert rules + recording rules — T3-04
