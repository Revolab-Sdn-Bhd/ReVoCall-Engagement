use anyhow::Result;
use prometheus::{IntGaugeVec, Opts, Registry};

use crate::config::RegistryAdapter;

pub struct Metrics {
    pub registry: Registry,
    pub registry_adapter_kind: IntGaugeVec,
}

impl Metrics {
    pub fn new(active: RegistryAdapter) -> Result<Self> {
        let registry = Registry::new();

        let registry_adapter_kind = IntGaugeVec::new(
            Opts::new(
                "engagementhub_registry_adapter_kind",
                "Active Registry adapter implementation (1 for the active kind, 0 for others)",
            ),
            &["kind"],
        )?;
        registry.register(Box::new(registry_adapter_kind.clone()))?;

        // Initialize both labels so absence ≠ "kind not yet observed".
        registry_adapter_kind.with_label_values(&["stub"]).set(0);
        registry_adapter_kind.with_label_values(&["grpc"]).set(0);
        registry_adapter_kind
            .with_label_values(&[active.as_metric_label()])
            .set(1);

        Ok(Self {
            registry,
            registry_adapter_kind,
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
    fn active_kind_is_one_others_zero() {
        let m = Metrics::new(RegistryAdapter::Stub).unwrap();
        let text = m.gather_text().unwrap();
        assert!(
            text.contains(r#"engagementhub_registry_adapter_kind{kind="stub"} 1"#),
            "missing active=stub line in:\n{text}"
        );
        assert!(
            text.contains(r#"engagementhub_registry_adapter_kind{kind="grpc"} 0"#),
            "missing inactive=grpc line in:\n{text}"
        );
    }
}
