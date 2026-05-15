use anyhow::Result;
use prometheus::{Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry};

use crate::config::{Env, RegistryAdapter};

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
    // --- Gauges ---
    pub active_engagements: IntGaugeVec,
    pub active_watches: IntGaugeVec,
    pub in_flight_invocations: IntGauge,
    pub db_pool_in_use: IntGauge,
    pub db_pool_idle: IntGauge,
    pub reconciler_backlog: IntGaugeVec,
}

impl Metrics {
    pub fn new(active: RegistryAdapter, env: Env, idle_mode: bool) -> Result<Self> {
        let registry = Registry::new();

        let registry_adapter_kind = IntGaugeVec::new(
            Opts::new(
                "engagementhub_registry_adapter_kind",
                "Active Registry adapter implementation (1 for the active kind, 0 for others)",
            ),
            &["kind", "env", "idle_mode"],
        )?;
        registry.register(Box::new(registry_adapter_kind.clone()))?;

        // Pre-initialize the active combination only (single-replica static fact).
        let idle_label = if idle_mode { "true" } else { "false" };
        registry_adapter_kind
            .with_label_values(&[active.as_metric_label(), env.as_metric_label(), idle_label])
            .set(1);

        let otel_exporter_dropped_spans = IntCounterVec::new(
            Opts::new(
                "engagementhub_otel_exporter_dropped_spans_total",
                "Spans dropped by each OTEL exporter due to queue-full or export error",
            ),
            &["exporter"],
        )?;
        registry.register(Box::new(otel_exporter_dropped_spans.clone()))?;
        // Pre-initialize all three so they appear in metrics output at zero
        for name in ["grafana", "langfuse", "local"] {
            otel_exporter_dropped_spans.with_label_values(&[name]);
        }

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

        // --- Gauges ---
        let active_engagements = IntGaugeVec::new(
            Opts::new(
                "engagementhub_active_engagements",
                "Current number of active engagements by status",
            ),
            &["status"],
        )?;
        registry.register(Box::new(active_engagements.clone()))?;

        let active_watches = IntGaugeVec::new(
            Opts::new(
                "engagementhub_active_watches",
                "Current number of active watch streams by filter type",
            ),
            &["filter_type"],
        )?;
        registry.register(Box::new(active_watches.clone()))?;

        let in_flight_invocations = IntGauge::new(
            "engagementhub_in_flight_invocations",
            "Current number of in-flight adapter invocations",
        )?;
        registry.register(Box::new(in_flight_invocations.clone()))?;

        let db_pool_in_use = IntGauge::new(
            "engagementhub_db_pool_in_use",
            "Current number of database pool connections in use",
        )?;
        registry.register(Box::new(db_pool_in_use.clone()))?;

        let db_pool_idle = IntGauge::new(
            "engagementhub_db_pool_idle",
            "Current number of idle database pool connections",
        )?;
        registry.register(Box::new(db_pool_idle.clone()))?;

        let reconciler_backlog = IntGaugeVec::new(
            Opts::new(
                "engagementhub_reconciler_backlog",
                "Current reconciler backlog depth by engagement class",
            ),
            &["class"],
        )?;
        registry.register(Box::new(reconciler_backlog.clone()))?;
        // Pre-initialize all 4 class values so they appear at zero immediately
        for class in [
            "pending_engagement",
            "orphan_compensation",
            "pending_audit",
            "overrun_live",
        ] {
            reconciler_backlog.with_label_values(&[class]);
        }

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
    }

    pub fn gather_text(&self) -> Result<String> {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buf = Vec::new();
        encoder.encode(&metric_families, &mut buf)?;
        Ok(String::from_utf8(buf)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_counters_registered() {
        let m = Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap();
        // Touch each labeled counter so its family appears in gather_text output.
        // prometheus 0.13 omits empty Vec families; this test-only Metrics instance
        // never reaches a real registry, so these sentinel values have no side effects.
        m.rpc_total.with_label_values(&["_", "_", "_"]);
        m.engagements_started_total.with_label_values(&["_", "_"]);
        m.engagements_terminal_total.with_label_values(&["_"]);
        m.engagement_errors_total.with_label_values(&["_"]);
        m.reconciler_swept_total.with_label_values(&["_", "_"]);
        m.adapter_retries_total.with_label_values(&["_", "_", "_"]);
        m.saga_compensation_outcome_total.with_label_values(&["_", "_"]);
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
        // Touch labeled histograms with no static pre-init so their families
        // appear in gather_text output. Same reasoning as counter Vec touches above —
        // test-only throwaway instance, no production side effects.
        m.rpc_duration_seconds.with_label_values(&["_", "_"]);
        m.adapter_duration_seconds.with_label_values(&["_", "_", "_"]);
        m.call_duration_seconds.with_label_values(&["_"]);
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
        // Touch labeled gauge Vecs so their families appear in gather_text output.
        // prometheus 0.13 omits empty Vec families; these sentinel values have no
        // side effects (test-only throwaway instance).
        m.active_engagements.with_label_values(&["_"]);
        m.active_watches.with_label_values(&["_"]);
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
