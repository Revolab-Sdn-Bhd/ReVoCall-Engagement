use std::sync::Arc;

use anyhow::Result;
use prometheus::{IntCounterVec, Opts, Registry};

pub struct AdapterMetrics {
    pub retries_total: IntCounterVec,
    pub deadline_exceeded_total: IntCounterVec,
}

impl AdapterMetrics {
    pub fn new(registry: &Registry) -> Result<Arc<Self>> {
        let retries_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_adapter_retries_total",
                "Retry attempts per adapter target and attempt number",
            ),
            &["target", "attempt"],
        )?;
        registry.register(Box::new(retries_total.clone()))?;

        let deadline_exceeded_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_deadline_exceeded_total",
                "Adapter calls refused due to deadline too close",
            ),
            &["target"],
        )?;
        registry.register(Box::new(deadline_exceeded_total.clone()))?;

        Ok(Arc::new(Self {
            retries_total,
            deadline_exceeded_total,
        }))
    }

    /// Returns a metrics instance backed by a throwaway registry (for tests).
    pub fn for_test() -> Arc<Self> {
        Self::new(&Registry::new()).expect("test metrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_both_counters() {
        let r = Registry::new();
        let m = AdapterMetrics::new(&r).unwrap();
        m.retries_total.with_label_values(&["registry", "1"]).inc();
        m.deadline_exceeded_total
            .with_label_values(&["registry"])
            .inc();
        use prometheus::Encoder;
        let enc = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        enc.encode(&r.gather(), &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("engagementhub_adapter_retries_total"));
        assert!(text.contains("engagementhub_deadline_exceeded_total"));
    }
}
