use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::AnalyticsError,
    ports::AnalyticsPort,
    types::{
        Analytics, GetAgentAnalyticsReq, GetAgentMetricsReq, GetOrgAnalyticsReq, GetOrgMetricsReq,
        Metrics,
    },
};
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::{
    metrics::AdapterMetrics,
    policies::{DEFAULT_RETRY, with_retry},
};

// ---------------------------------------------------------------------------
// Private downstream response shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AnalyticsResp {
    average_conversation_duration: f64,
    containment_rate: f64,
    customer_satisfaction_rate: f64,
    dropoff_rate: f64,
    escalation_rate: f64,
    total_inquiries: u32,
    #[serde(default)]
    category_counts: HashMap<String, u32>,
}

#[derive(Deserialize)]
struct MetricData {
    categories: Vec<String>,
    series: Vec<f64>,
}

#[derive(Deserialize)]
struct MetricsResp {
    data: MetricData,
}

// ---------------------------------------------------------------------------
// HTTP helper
// ---------------------------------------------------------------------------

fn map_http_status(status: StatusCode, body: &str) -> AnalyticsError {
    match status {
        StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            AnalyticsError::Permanent(format!("{status}: {body}"))
        }
        StatusCode::SERVICE_UNAVAILABLE => AnalyticsError::Unavailable,
        _ => AnalyticsError::Transient(format!("{status}: {body}")),
    }
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

pub struct AnalyticsHttpAdapter {
    client: Client,
    base_url: String,
    metrics: Arc<AdapterMetrics>,
}

impl AnalyticsHttpAdapter {
    pub fn new(client: Client, base_url: String, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client,
            base_url,
            metrics,
        }
    }
}

#[async_trait]
impl AnalyticsPort for AnalyticsHttpAdapter {
    async fn get_agent_analytics(
        &self,
        req: GetAgentAnalyticsReq,
    ) -> Result<Analytics, AnalyticsError> {
        let base = format!("{}/calls/agents/{}/analytics", self.base_url, req.agent_id);
        let mut qp: Vec<(&str, String)> = vec![];
        if let Some(v) = req.metric {
            qp.push(("metric", v))
        }
        if let Some(v) = req.granularity {
            qp.push(("granularity", v))
        }
        if let Some(v) = req.start_date {
            qp.push(("startDate", v))
        }
        if let Some(v) = req.end_date {
            qp.push(("endDate", v))
        }
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, None, "analytics", Some(&m), move || {
            let c = client.clone();
            let b = base.clone();
            let params = qp.clone();
            async move {
                let resp = c
                    .get(&b)
                    .query(&params)
                    .send()
                    .await
                    .map_err(|e| AnalyticsError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let r: AnalyticsResp = resp
                        .json()
                        .await
                        .map_err(|e| AnalyticsError::Permanent(e.to_string()))?;
                    Ok(Analytics {
                        average_conversation_duration: r.average_conversation_duration,
                        containment_rate: r.containment_rate,
                        customer_satisfaction_rate: r.customer_satisfaction_rate,
                        dropoff_rate: r.dropoff_rate,
                        escalation_rate: r.escalation_rate,
                        total_inquiries: r.total_inquiries,
                        category_counts: r.category_counts,
                    })
                } else {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(status, &body))
                }
            }
        })
        .await
    }

    async fn get_agent_metrics(&self, req: GetAgentMetricsReq) -> Result<Metrics, AnalyticsError> {
        let base = format!("{}/calls/agents/{}/metrics", self.base_url, req.agent_id);
        let mut qp: Vec<(&str, String)> = vec![];
        if let Some(v) = req.metric {
            qp.push(("metric", v))
        }
        if let Some(v) = req.granularity {
            qp.push(("granularity", v))
        }
        if let Some(v) = req.start_date {
            qp.push(("startDate", v))
        }
        if let Some(v) = req.end_date {
            qp.push(("endDate", v))
        }
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, None, "analytics", Some(&m), move || {
            let c = client.clone();
            let b = base.clone();
            let params = qp.clone();
            async move {
                let resp = c
                    .get(&b)
                    .query(&params)
                    .send()
                    .await
                    .map_err(|e| AnalyticsError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let r: MetricsResp = resp
                        .json()
                        .await
                        .map_err(|e| AnalyticsError::Permanent(e.to_string()))?;
                    Ok(Metrics {
                        categories: r.data.categories,
                        series: r.data.series,
                    })
                } else {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(status, &body))
                }
            }
        })
        .await
    }

    async fn get_org_analytics(
        &self,
        req: GetOrgAnalyticsReq,
    ) -> Result<Analytics, AnalyticsError> {
        let base = format!(
            "{}/calls/organizations/{}/analytics",
            self.base_url, req.org_id
        );
        let mut qp: Vec<(&str, String)> = vec![];
        if let Some(v) = req.metric {
            qp.push(("metric", v))
        }
        if let Some(v) = req.granularity {
            qp.push(("granularity", v))
        }
        if let Some(v) = req.start_date {
            qp.push(("startDate", v))
        }
        if let Some(v) = req.end_date {
            qp.push(("endDate", v))
        }
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, None, "analytics", Some(&m), move || {
            let c = client.clone();
            let b = base.clone();
            let params = qp.clone();
            async move {
                let resp = c
                    .get(&b)
                    .query(&params)
                    .send()
                    .await
                    .map_err(|e| AnalyticsError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let r: AnalyticsResp = resp
                        .json()
                        .await
                        .map_err(|e| AnalyticsError::Permanent(e.to_string()))?;
                    Ok(Analytics {
                        average_conversation_duration: r.average_conversation_duration,
                        containment_rate: r.containment_rate,
                        customer_satisfaction_rate: r.customer_satisfaction_rate,
                        dropoff_rate: r.dropoff_rate,
                        escalation_rate: r.escalation_rate,
                        total_inquiries: r.total_inquiries,
                        category_counts: r.category_counts,
                    })
                } else {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(status, &body))
                }
            }
        })
        .await
    }

    async fn get_org_metrics(&self, req: GetOrgMetricsReq) -> Result<Metrics, AnalyticsError> {
        let base = format!(
            "{}/calls/organizations/{}/metrics",
            self.base_url, req.org_id
        );
        let mut qp: Vec<(&str, String)> = vec![];
        if let Some(v) = req.metric {
            qp.push(("metric", v))
        }
        if let Some(v) = req.granularity {
            qp.push(("granularity", v))
        }
        if let Some(v) = req.start_date {
            qp.push(("startDate", v))
        }
        if let Some(v) = req.end_date {
            qp.push(("endDate", v))
        }
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, None, "analytics", Some(&m), move || {
            let c = client.clone();
            let b = base.clone();
            let params = qp.clone();
            async move {
                let resp = c
                    .get(&b)
                    .query(&params)
                    .send()
                    .await
                    .map_err(|e| AnalyticsError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let r: MetricsResp = resp
                        .json()
                        .await
                        .map_err(|e| AnalyticsError::Permanent(e.to_string()))?;
                    Ok(Metrics {
                        categories: r.data.categories,
                        series: r.data.series,
                    })
                } else {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    Err(map_http_status(status, &body))
                }
            }
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Tests — wiremock
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path_regex},
    };

    async fn make_adapter() -> (MockServer, AnalyticsHttpAdapter) {
        let server = MockServer::start().await;
        let adapter =
            AnalyticsHttpAdapter::new(Client::new(), server.uri(), AdapterMetrics::for_test());
        (server, adapter)
    }

    fn analytics_json() -> serde_json::Value {
        serde_json::json!({
            "average_conversation_duration": 45.5,
            "containment_rate": 0.75,
            "customer_satisfaction_rate": 0.85,
            "dropoff_rate": 0.1,
            "escalation_rate": 0.05,
            "total_inquiries": 100,
            "category_counts": {}
        })
    }

    fn metrics_json() -> serde_json::Value {
        serde_json::json!({"data": {"categories": ["mon","tue"], "series": [1.0, 2.0]}})
    }

    #[tokio::test]
    async fn get_agent_analytics_success() {
        let (server, adapter) = make_adapter().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/calls/agents/a1/analytics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(analytics_json()))
            .mount(&server)
            .await;
        let a = adapter
            .get_agent_analytics(GetAgentAnalyticsReq {
                agent_id: "a1".into(),
                metric: None,
                granularity: None,
                start_date: None,
                end_date: None,
            })
            .await
            .expect("ok");
        assert_eq!(a.total_inquiries, 100);
    }

    #[tokio::test]
    async fn get_agent_metrics_unwraps_data() {
        let (server, adapter) = make_adapter().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/calls/agents/a1/metrics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(metrics_json()))
            .mount(&server)
            .await;
        let m = adapter
            .get_agent_metrics(GetAgentMetricsReq {
                agent_id: "a1".into(),
                metric: None,
                granularity: None,
                start_date: None,
                end_date: None,
            })
            .await
            .expect("ok");
        assert_eq!(m.categories, vec!["mon", "tue"]);
        assert_eq!(m.series.len(), 2);
    }

    #[tokio::test]
    async fn get_org_analytics_success() {
        let (server, adapter) = make_adapter().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/calls/organizations/org1/analytics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(analytics_json()))
            .mount(&server)
            .await;
        let a = adapter
            .get_org_analytics(GetOrgAnalyticsReq {
                org_id: "org1".into(),
                metric: None,
                granularity: None,
                start_date: None,
                end_date: None,
            })
            .await
            .expect("ok");
        assert!((a.containment_rate - 0.75).abs() < 1e-9);
    }

    #[tokio::test]
    async fn get_org_metrics_success() {
        let (server, adapter) = make_adapter().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/calls/organizations/org1/metrics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(metrics_json()))
            .mount(&server)
            .await;
        let m = adapter
            .get_org_metrics(GetOrgMetricsReq {
                org_id: "org1".into(),
                metric: None,
                granularity: None,
                start_date: None,
                end_date: None,
            })
            .await
            .expect("ok");
        assert_eq!(m.series, vec![1.0, 2.0]);
    }

    #[tokio::test]
    async fn not_found_maps_to_permanent() {
        let (server, adapter) = make_adapter().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/calls/agents/missing/analytics"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let err = adapter
            .get_agent_analytics(GetAgentAnalyticsReq {
                agent_id: "missing".into(),
                metric: None,
                granularity: None,
                start_date: None,
                end_date: None,
            })
            .await
            .expect_err("fail");
        assert!(matches!(err, AnalyticsError::Permanent(_)));
    }
}
