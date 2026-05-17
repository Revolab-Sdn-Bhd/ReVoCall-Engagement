//! Fake implementation of [`AnalyticsPort`] for testing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::Outcome;
use crate::{
    error::AnalyticsError,
    ports::AnalyticsPort,
    types::{
        Analytics, GetAgentAnalyticsReq, GetAgentMetricsReq, GetOrgAnalyticsReq, GetOrgMetricsReq,
        Metrics,
    },
};

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct FakeAnalyticsInner {
    get_agent_analytics: VecDeque<Outcome<Analytics>>,
    get_agent_metrics: VecDeque<Outcome<Metrics>>,
    get_org_analytics: VecDeque<Outcome<Analytics>>,
    get_org_metrics: VecDeque<Outcome<Metrics>>,
}

// ---------------------------------------------------------------------------
// Public fake
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FakeAnalyticsPort {
    inner: Arc<Mutex<FakeAnalyticsInner>>,
}

impl FakeAnalyticsPort {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeAnalyticsInner {
                get_agent_analytics: VecDeque::new(),
                get_agent_metrics: VecDeque::new(),
                get_org_analytics: VecDeque::new(),
                get_org_metrics: VecDeque::new(),
            })),
        }
    }

    pub fn push_get_agent_analytics(&self, outcome: Outcome<Analytics>) {
        self.inner
            .lock()
            .unwrap()
            .get_agent_analytics
            .push_back(outcome);
    }

    pub fn push_get_agent_metrics(&self, outcome: Outcome<Metrics>) {
        self.inner
            .lock()
            .unwrap()
            .get_agent_metrics
            .push_back(outcome);
    }

    pub fn push_get_org_analytics(&self, outcome: Outcome<Analytics>) {
        self.inner
            .lock()
            .unwrap()
            .get_org_analytics
            .push_back(outcome);
    }

    pub fn push_get_org_metrics(&self, outcome: Outcome<Metrics>) {
        self.inner
            .lock()
            .unwrap()
            .get_org_metrics
            .push_back(outcome);
    }
}

impl Default for FakeAnalyticsPort {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AnalyticsPort for FakeAnalyticsPort {
    async fn get_agent_analytics(
        &self,
        _req: GetAgentAnalyticsReq,
    ) -> Result<Analytics, AnalyticsError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_agent_analytics
            .pop_front()
            .expect("FakeAnalyticsPort::get_agent_analytics has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(AnalyticsError::Transient(msg)),
            Outcome::Permanent(msg) => Err(AnalyticsError::Permanent(msg)),
            Outcome::Unavailable => Err(AnalyticsError::Unavailable),
            Outcome::Panic => panic!("FakeAnalyticsPort::get_agent_analytics panic injected"),
        }
    }

    async fn get_agent_metrics(&self, _req: GetAgentMetricsReq) -> Result<Metrics, AnalyticsError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_agent_metrics
            .pop_front()
            .expect("FakeAnalyticsPort::get_agent_metrics has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(AnalyticsError::Transient(msg)),
            Outcome::Permanent(msg) => Err(AnalyticsError::Permanent(msg)),
            Outcome::Unavailable => Err(AnalyticsError::Unavailable),
            Outcome::Panic => panic!("FakeAnalyticsPort::get_agent_metrics panic injected"),
        }
    }

    async fn get_org_analytics(
        &self,
        _req: GetOrgAnalyticsReq,
    ) -> Result<Analytics, AnalyticsError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_org_analytics
            .pop_front()
            .expect("FakeAnalyticsPort::get_org_analytics has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(AnalyticsError::Transient(msg)),
            Outcome::Permanent(msg) => Err(AnalyticsError::Permanent(msg)),
            Outcome::Unavailable => Err(AnalyticsError::Unavailable),
            Outcome::Panic => panic!("FakeAnalyticsPort::get_org_analytics panic injected"),
        }
    }

    async fn get_org_metrics(&self, _req: GetOrgMetricsReq) -> Result<Metrics, AnalyticsError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_org_metrics
            .pop_front()
            .expect("FakeAnalyticsPort::get_org_metrics has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(AnalyticsError::Transient(msg)),
            Outcome::Permanent(msg) => Err(AnalyticsError::Permanent(msg)),
            Outcome::Unavailable => Err(AnalyticsError::Unavailable),
            Outcome::Panic => panic!("FakeAnalyticsPort::get_org_metrics panic injected"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_analytics() -> Analytics {
        Analytics {
            average_conversation_duration: 0.0,
            containment_rate: 0.0,
            customer_satisfaction_rate: 0.0,
            dropoff_rate: 0.0,
            escalation_rate: 0.0,
            total_inquiries: 0,
            category_counts: std::collections::HashMap::new(),
        }
    }

    fn empty_metrics() -> Metrics {
        Metrics {
            categories: vec![],
            series: vec![],
        }
    }

    fn agent_analytics_req() -> GetAgentAnalyticsReq {
        GetAgentAnalyticsReq {
            agent_id: "agent-1".into(),
            metric: None,
            granularity: None,
            start_date: None,
            end_date: None,
        }
    }

    fn agent_metrics_req() -> GetAgentMetricsReq {
        GetAgentMetricsReq {
            agent_id: "agent-1".into(),
            metric: None,
            granularity: None,
            start_date: None,
            end_date: None,
        }
    }

    fn org_analytics_req() -> GetOrgAnalyticsReq {
        GetOrgAnalyticsReq {
            org_id: "org-1".into(),
            metric: None,
            granularity: None,
            start_date: None,
            end_date: None,
        }
    }

    fn org_metrics_req() -> GetOrgMetricsReq {
        GetOrgMetricsReq {
            org_id: "org-1".into(),
            metric: None,
            granularity: None,
            start_date: None,
            end_date: None,
        }
    }

    // --- get_agent_analytics ---

    #[tokio::test]
    async fn get_agent_analytics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Success(empty_analytics()));
        let result = fake.get_agent_analytics(agent_analytics_req()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_agent_analytics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Transient("timeout".into()));
        let result = fake.get_agent_analytics(agent_analytics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_agent_analytics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_agent_analytics(agent_analytics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_agent_analytics_unavailable() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Unavailable);
        let result = fake.get_agent_analytics(agent_analytics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Unavailable)));
    }

    #[tokio::test]
    async fn get_agent_analytics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Panic);
        let result =
            tokio::task::spawn(
                async move { fake.get_agent_analytics(agent_analytics_req()).await },
            )
            .await;
        assert!(result.unwrap_err().is_panic());
    }

    // --- get_agent_metrics ---

    #[tokio::test]
    async fn get_agent_metrics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Success(empty_metrics()));
        let result = fake.get_agent_metrics(agent_metrics_req()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_agent_metrics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Transient("timeout".into()));
        let result = fake.get_agent_metrics(agent_metrics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_agent_metrics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_agent_metrics(agent_metrics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_agent_metrics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Panic);
        let result =
            tokio::task::spawn(async move { fake.get_agent_metrics(agent_metrics_req()).await })
                .await;
        assert!(result.unwrap_err().is_panic());
    }

    // --- get_org_analytics ---

    #[tokio::test]
    async fn get_org_analytics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Success(empty_analytics()));
        let result = fake.get_org_analytics(org_analytics_req()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_org_analytics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Transient("timeout".into()));
        let result = fake.get_org_analytics(org_analytics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_org_analytics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_org_analytics(org_analytics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_org_analytics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Panic);
        let result =
            tokio::task::spawn(async move { fake.get_org_analytics(org_analytics_req()).await })
                .await;
        assert!(result.unwrap_err().is_panic());
    }

    // --- get_org_metrics ---

    #[tokio::test]
    async fn get_org_metrics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Success(empty_metrics()));
        let result = fake.get_org_metrics(org_metrics_req()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_org_metrics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Transient("timeout".into()));
        let result = fake.get_org_metrics(org_metrics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_org_metrics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_org_metrics(org_metrics_req()).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_org_metrics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Panic);
        let result =
            tokio::task::spawn(async move { fake.get_org_metrics(org_metrics_req()).await }).await;
        assert!(result.unwrap_err().is_panic());
    }

    #[tokio::test]
    async fn test_get_agent_analytics_queue_ordering() {
        let fake = FakeAnalyticsPort::new();
        // Push transient first, then success
        fake.push_get_agent_analytics(Outcome::Transient("first".into()));
        fake.push_get_agent_analytics(Outcome::Success(empty_analytics()));

        // First call should be transient
        let first = fake.get_agent_analytics(agent_analytics_req()).await;
        assert!(matches!(first, Err(AnalyticsError::Transient(ref msg)) if msg == "first"));

        // Second call should be success
        let second = fake.get_agent_analytics(agent_analytics_req()).await;
        assert!(second.is_ok());
    }

    #[tokio::test]
    async fn test_get_agent_analytics_empty_queue_panics() {
        let fake = FakeAnalyticsPort::new();
        // Don't push anything
        let result =
            tokio::task::spawn(
                async move { fake.get_agent_analytics(agent_analytics_req()).await },
            )
            .await;
        assert!(result.unwrap_err().is_panic());
    }
}
