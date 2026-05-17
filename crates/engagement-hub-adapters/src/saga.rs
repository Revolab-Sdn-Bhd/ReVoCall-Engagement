//! Saga compensation observability primitives.
//!
//! These enums identify the stage (which downstream — VM or JM) and the
//! outcome of a compensation attempt for the `engagementhub_saga_compensation_outcome_total`
//! Prometheus counter (PRD §7).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationStage {
    /// Compensating after a failed StartEngagement bind by cancelling the
    /// journey execution that did succeed.
    JmCancel,
    /// Compensating after a failed StartEngagement bind by stopping the
    /// voice session that did succeed.
    VmStop,
}

impl CompensationStage {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::JmCancel => "jm_cancel",
            Self::VmStop => "vm_stop",
        }
    }

    pub const ALL: [Self; 2] = [Self::JmCancel, Self::VmStop];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationOutcome {
    Success,
    TransientFailureRetried,
    ExhaustedToReconciler,
    NoCompensationNeeded,
}

impl CompensationOutcome {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::TransientFailureRetried => "transient_failure_retried",
            Self::ExhaustedToReconciler => "exhausted_to_reconciler",
            Self::NoCompensationNeeded => "no_compensation_needed",
        }
    }

    pub const ALL: [Self; 4] = [
        Self::Success,
        Self::TransientFailureRetried,
        Self::ExhaustedToReconciler,
        Self::NoCompensationNeeded,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_labels_match_prd_spec() {
        assert_eq!(CompensationStage::JmCancel.as_label(), "jm_cancel");
        assert_eq!(CompensationStage::VmStop.as_label(), "vm_stop");
    }

    #[test]
    fn outcome_labels_match_prd_spec() {
        assert_eq!(CompensationOutcome::Success.as_label(), "success");
        assert_eq!(
            CompensationOutcome::TransientFailureRetried.as_label(),
            "transient_failure_retried"
        );
        assert_eq!(
            CompensationOutcome::ExhaustedToReconciler.as_label(),
            "exhausted_to_reconciler"
        );
        assert_eq!(
            CompensationOutcome::NoCompensationNeeded.as_label(),
            "no_compensation_needed"
        );
    }

    #[test]
    fn all_combinations_covered() {
        assert_eq!(CompensationStage::ALL.len(), 2);
        assert_eq!(CompensationOutcome::ALL.len(), 4);
    }
}
