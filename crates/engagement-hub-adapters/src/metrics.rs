use std::sync::Arc;

use anyhow::Result;
use prometheus::{IntCounterVec, Opts, Registry};

use crate::saga::{CompensationOutcome, CompensationStage};

pub struct AdapterMetrics {
    pub retries_total: IntCounterVec,
    pub deadline_exceeded_total: IntCounterVec,
    pub saga_compensation_outcome_total: IntCounterVec,
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

        let saga_compensation_outcome_total = IntCounterVec::new(
            Opts::new(
                "engagementhub_saga_compensation_outcome_total",
                "Saga compensation attempt outcomes by stage (PRD §7)",
            ),
            &["stage", "result"],
        )?;
        registry.register(Box::new(saga_compensation_outcome_total.clone()))?;

        // Zero-init all 2 × 4 series so dashboards/alerts that assume series
        // presence don't break before the first real increment.
        for stage in CompensationStage::ALL {
            for outcome in CompensationOutcome::ALL {
                saga_compensation_outcome_total
                    .with_label_values(&[stage.as_label(), outcome.as_label()])
                    .inc_by(0);
            }
        }

        Ok(Arc::new(Self {
            retries_total,
            deadline_exceeded_total,
            saga_compensation_outcome_total,
        }))
    }

    pub fn record_compensation(&self, stage: CompensationStage, outcome: CompensationOutcome) {
        self.saga_compensation_outcome_total
            .with_label_values(&[stage.as_label(), outcome.as_label()])
            .inc();
    }

    /// Returns a metrics instance backed by a throwaway registry (for tests).
    pub fn for_test() -> Arc<Self> {
        Self::new(&Registry::new()).expect("test metrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saga::{CompensationOutcome, CompensationStage};
    use prometheus::Encoder;

    fn gather_text(r: &Registry) -> String {
        let enc = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        enc.encode(&r.gather(), &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn registers_all_counters() {
        let r = Registry::new();
        let m = AdapterMetrics::new(&r).unwrap();
        m.retries_total.with_label_values(&["registry", "1"]).inc();
        m.deadline_exceeded_total
            .with_label_values(&["registry"])
            .inc();
        let text = gather_text(&r);
        assert!(text.contains("engagementhub_adapter_retries_total"));
        assert!(text.contains("engagementhub_deadline_exceeded_total"));
        assert!(text.contains("engagementhub_saga_compensation_outcome_total"));
    }

    #[test]
    fn saga_counter_is_zero_initialized_for_all_label_combinations() {
        let r = Registry::new();
        let _m = AdapterMetrics::new(&r).unwrap();
        let text = gather_text(&r);
        // All 2 × 4 = 8 series must be present with value 0.
        for stage in CompensationStage::ALL {
            for outcome in CompensationOutcome::ALL {
                let expected = format!(
                    "engagementhub_saga_compensation_outcome_total{{result=\"{}\",stage=\"{}\"}} 0",
                    outcome.as_label(),
                    stage.as_label()
                );
                assert!(
                    text.contains(&expected),
                    "missing zero-init for stage={:?} outcome={:?}\n--- gather text ---\n{}",
                    stage,
                    outcome,
                    text,
                );
            }
        }
    }

    #[test]
    fn record_compensation_increments_correct_series() {
        let r = Registry::new();
        let m = AdapterMetrics::new(&r).unwrap();
        m.record_compensation(CompensationStage::JmCancel, CompensationOutcome::Success);
        m.record_compensation(
            CompensationStage::VmStop,
            CompensationOutcome::ExhaustedToReconciler,
        );
        let text = gather_text(&r);
        assert!(text.contains(
            "engagementhub_saga_compensation_outcome_total{result=\"success\",stage=\"jm_cancel\"} 1"
        ));
        assert!(text.contains(
            "engagementhub_saga_compensation_outcome_total{result=\"exhausted_to_reconciler\",stage=\"vm_stop\"} 1"
        ));
    }
}
