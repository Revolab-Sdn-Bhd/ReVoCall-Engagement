use anyhow::Result;
use prometheus::{IntCounterVec, IntGaugeVec, Opts, Registry};

use crate::config::{Env, RegistryAdapter};

pub struct Metrics {
    pub registry: Registry,
    pub registry_adapter_kind: IntGaugeVec,
    pub otel_exporter_dropped_spans: IntCounterVec,
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

        Ok(Self {
            registry,
            registry_adapter_kind,
            otel_exporter_dropped_spans,
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
