//! Fake implementation of [`AnalyticsPort`] for testing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{
    error::AnalyticsError,
    ports::AnalyticsPort,
    types::{
        Analytics, GetAgentAnalyticsReq, GetAgentMetricsReq, GetOrgAnalyticsReq,
        GetOrgMetricsReq, Metrics,
    },
};
use super::Outcome;

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
        self.inner.lock().unwrap().get_agent_analytics.push_back(outcome);
    }

    pub fn push_get_agent_metrics(&self, outcome: Outcome<Metrics>) {
        self.inner.lock().unwrap().get_agent_metrics.push_back(outcome);
    }

    pub fn push_get_org_analytics(&self, outcome: Outcome<Analytics>) {
        self.inner.lock().unwrap().get_org_analytics.push_back(outcome);
    }

    pub fn push_get_org_metrics(&self, outcome: Outcome<Metrics>) {
        self.inner.lock().unwrap().get_org_metrics.push_back(outcome);
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

    async fn get_agent_metrics(
        &self,
        _req: GetAgentMetricsReq,
    ) -> Result<Metrics, AnalyticsError> {
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

    async fn get_org_metrics(
        &self,
        _req: GetOrgMetricsReq,
    ) -> Result<Metrics, AnalyticsError> {
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

    // --- get_agent_analytics ---

    #[tokio::test]
    async fn get_agent_analytics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Success(Analytics {}));
        let result = fake.get_agent_analytics(GetAgentAnalyticsReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_agent_analytics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Transient("timeout".into()));
        let result = fake.get_agent_analytics(GetAgentAnalyticsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_agent_analytics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_agent_analytics(GetAgentAnalyticsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_agent_analytics_unavailable() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Unavailable);
        let result = fake.get_agent_analytics(GetAgentAnalyticsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Unavailable)));
    }

    #[tokio::test]
    async fn get_agent_analytics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_analytics(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.get_agent_analytics(GetAgentAnalyticsReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- get_agent_metrics ---

    #[tokio::test]
    async fn get_agent_metrics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Success(Metrics {}));
        let result = fake.get_agent_metrics(GetAgentMetricsReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_agent_metrics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Transient("timeout".into()));
        let result = fake.get_agent_metrics(GetAgentMetricsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_agent_metrics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_agent_metrics(GetAgentMetricsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_agent_metrics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_agent_metrics(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.get_agent_metrics(GetAgentMetricsReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- get_org_analytics ---

    #[tokio::test]
    async fn get_org_analytics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Success(Analytics {}));
        let result = fake.get_org_analytics(GetOrgAnalyticsReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_org_analytics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Transient("timeout".into()));
        let result = fake.get_org_analytics(GetOrgAnalyticsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_org_analytics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_org_analytics(GetOrgAnalyticsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_org_analytics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_analytics(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.get_org_analytics(GetOrgAnalyticsReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- get_org_metrics ---

    #[tokio::test]
    async fn get_org_metrics_success() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Success(Metrics {}));
        let result = fake.get_org_metrics(GetOrgMetricsReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_org_metrics_transient() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Transient("timeout".into()));
        let result = fake.get_org_metrics(GetOrgMetricsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Transient(_))));
    }

    #[tokio::test]
    async fn get_org_metrics_permanent() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Permanent("forbidden".into()));
        let result = fake.get_org_metrics(GetOrgMetricsReq {}).await;
        assert!(matches!(result, Err(AnalyticsError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_org_metrics_panic() {
        let fake = FakeAnalyticsPort::new();
        fake.push_get_org_metrics(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.get_org_metrics(GetOrgMetricsReq {}).await
        })
        .await;
        assert!(result.is_err());
    }
}
