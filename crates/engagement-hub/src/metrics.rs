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
