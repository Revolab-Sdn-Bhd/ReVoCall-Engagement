use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::PostCallError,
    ports::PostCallPort,
    types::{
        CallLog, EngagementId, ListAgentCallLogsReq, ListOrgCallLogsReq, OutputExtraction,
        OutputField, Page, Sentiment, Summary, Transcript, TranscriptMessage,
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
struct TranscriptionItem {
    message: String,
    role: String,
    audio_url: Option<String>,
    emotion: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptionsResp {
    data: Vec<TranscriptionItem>,
    total_size: Option<i32>,
}

#[derive(Deserialize)]
struct SummaryData {
    summary: String,
    resolution: Option<String>,
    resolution_explanation: Option<String>,
}

#[derive(Deserialize)]
struct SummaryResp {
    data: Option<SummaryData>,
}

#[derive(Deserialize)]
struct SentimentData {
    sentiment: String,
    justification: String,
}

#[derive(Deserialize)]
struct SentimentResp {
    data: Option<SentimentData>,
}

#[derive(Deserialize)]
struct OutputExtractionItem {
    key: String,
    value: String,
}

#[derive(Deserialize)]
struct OutputExtractionResp {
    data: Vec<OutputExtractionItem>,
}

#[derive(Deserialize)]
struct CallLogItem {
    id: String,
    room_name: Option<String>,
    batch_id: Option<String>,
    duration: Option<i32>,
    identity: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct CallLogListResp {
    data: Option<Vec<CallLogItem>>,
    total_size: Option<u32>,
}

// ---------------------------------------------------------------------------
// HTTP helper
// ---------------------------------------------------------------------------

fn map_http_status(status: StatusCode, body: &str) -> PostCallError {
    match status {
        StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            PostCallError::Permanent(format!("{status}: {body}"))
        }
        StatusCode::SERVICE_UNAVAILABLE => PostCallError::Unavailable,
        _ => PostCallError::Transient(format!("{status}: {body}")),
    }
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: &str,
) -> Result<T, PostCallError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| PostCallError::Transient(e.to_string()))?;
    if resp.status().is_success() {
        resp.json::<T>()
            .await
            .map_err(|e| PostCallError::Permanent(e.to_string()))
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(map_http_status(status, &body))
    }
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

pub struct PostCallHttpAdapter {
    client: Client,
    base_url: String,
    metrics: Arc<AdapterMetrics>,
}

impl PostCallHttpAdapter {
    pub fn new(client: Client, base_url: String, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client,
            base_url,
            metrics,
        }
    }
}

#[async_trait]
impl PostCallPort for PostCallHttpAdapter {
    async fn get_transcript(&self, eng: &EngagementId) -> Result<Transcript, PostCallError> {
        let url = format!("{}/calls/{}/transcription", self.base_url, eng);
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
            let c = client.clone();
            let u = url.clone();
            async move {
                let r: TranscriptionsResp = get_json(&c, &u).await?;
                Ok(Transcript {
                    messages: r
                        .data
                        .into_iter()
                        .map(|i| TranscriptMessage {
                            message: i.message,
                            role: i.role,
                            audio_url: i.audio_url,
                            emotion: i.emotion,
                        })
                        .collect(),
                    total_size: r.total_size.unwrap_or(0),
                })
            }
        })
        .await
    }

    async fn get_summary(&self, eng: &EngagementId) -> Result<Summary, PostCallError> {
        let url = format!("{}/calls/{}/summary", self.base_url, eng);
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
            let c = client.clone();
            let u = url.clone();
            async move {
                let r: SummaryResp = get_json(&c, &u).await?;
                let d = r
                    .data
                    .ok_or_else(|| PostCallError::Permanent("empty summary data".into()))?;
                Ok(Summary {
                    summary: d.summary,
                    resolution: d.resolution,
                    resolution_explanation: d.resolution_explanation,
                })
            }
        })
        .await
    }

    async fn get_sentiment(&self, eng: &EngagementId) -> Result<Sentiment, PostCallError> {
        let url = format!("{}/calls/{}/sentiment", self.base_url, eng);
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
            let c = client.clone();
            let u = url.clone();
            async move {
                let r: SentimentResp = get_json(&c, &u).await?;
                let d = r
                    .data
                    .ok_or_else(|| PostCallError::Permanent("empty sentiment data".into()))?;
                Ok(Sentiment {
                    label: d.sentiment,
                    justification: d.justification,
                })
            }
        })
        .await
    }

    async fn get_output_extraction(
        &self,
        eng: &EngagementId,
    ) -> Result<OutputExtraction, PostCallError> {
        let url = format!("{}/calls/{}/state", self.base_url, eng);
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
            let c = client.clone();
            let u = url.clone();
            async move {
                let r: OutputExtractionResp = get_json(&c, &u).await?;
                Ok(OutputExtraction {
                    fields: r
                        .data
                        .into_iter()
                        .map(|f| OutputField {
                            key: f.key,
                            value: f.value,
                        })
                        .collect(),
                })
            }
        })
        .await
    }

    async fn list_agent_call_logs(
        &self,
        req: ListAgentCallLogsReq,
    ) -> Result<Page<CallLog>, PostCallError> {
        let base_url = format!("{}/calls/{}/history-call", self.base_url, req.agent_id);
        let mut qp: Vec<(&str, String)> = vec![];
        if let Some(v) = req.skip {
            qp.push(("skip", v.to_string()))
        }
        if let Some(v) = req.limit {
            qp.push(("limit", v.to_string()))
        }
        if let Some(v) = req.start_date {
            qp.push(("start_date", v))
        }
        if let Some(v) = req.end_date {
            qp.push(("end_date", v))
        }
        if let Some(v) = req.identity {
            qp.push(("identity", v))
        }
        if let Some(v) = req.id {
            qp.push(("id", v))
        }
        if let Some(v) = req.batch_id {
            qp.push(("batch_id", v))
        }
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
            let c = client.clone();
            let base = base_url.clone();
            let params = qp.clone();
            async move {
                let resp = c
                    .get(&base)
                    .query(&params)
                    .send()
                    .await
                    .map_err(|e| PostCallError::Transient(e.to_string()))?;
                if resp.status().is_success() {
                    let r: CallLogListResp = resp
                        .json()
                        .await
                        .map_err(|e| PostCallError::Permanent(e.to_string()))?;
                    Ok(Page {
                        items: r
                            .data
                            .unwrap_or_default()
                            .into_iter()
                            .map(|l| CallLog {
                                id: l.id,
                                room_name: l.room_name,
                                batch_id: l.batch_id,
                                duration: l.duration,
                                identity: l.identity,
                                created_at: l.created_at,
                            })
                            .collect(),
                        total_size: r.total_size,
                        next_page_token: None,
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

    async fn list_org_call_logs(
        &self,
        req: ListOrgCallLogsReq,
    ) -> Result<Page<CallLog>, PostCallError> {
        let mut params = vec![];
        if let Some(v) = req.skip {
            params.push(format!("skip={v}"))
        }
        if let Some(v) = req.limit {
            params.push(format!("limit={v}"))
        }
        if let Some(v) = &req.start_date {
            params.push(format!("start_date={v}"))
        }
        if let Some(v) = &req.end_date {
            params.push(format!("end_date={v}"))
        }
        if let Some(v) = &req.contact_number {
            params.push(format!("contact_number={v}"))
        }
        if let Some(v) = &req.call_id {
            params.push(format!("call_id={v}"))
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        let url = format!("{}/calls/organizations/{}{}", self.base_url, req.org_id, qs);
        let client = self.client.clone();
        let m = self.metrics.clone();
        with_retry(DEFAULT_RETRY, "post_call", Some(&m), move || {
            let c = client.clone();
            let u = url.clone();
            async move {
                let r: CallLogListResp = get_json(&c, &u).await?;
                Ok(Page {
                    items: r
                        .data
                        .unwrap_or_default()
                        .into_iter()
                        .map(|l| CallLog {
                            id: l.id,
                            room_name: l.room_name,
                            batch_id: l.batch_id,
                            duration: l.duration,
                            identity: l.identity,
                            created_at: l.created_at,
                        })
                        .collect(),
                    total_size: r.total_size,
                    next_page_token: None,
                })
            }
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Tests — wiremock HTTP mocks
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engagement_hub_ports::types::EngagementId;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, path_regex},
    };

    async fn make_adapter() -> (MockServer, PostCallHttpAdapter) {
        let server = MockServer::start().await;
        let adapter =
            PostCallHttpAdapter::new(Client::new(), server.uri(), AdapterMetrics::for_test());
        (server, adapter)
    }

    #[tokio::test]
    async fn get_transcript_maps_messages() {
        let (server, adapter) = make_adapter().await;
        let eng = EngagementId::new();
        Mock::given(method("GET"))
            .and(path(format!("/calls/{eng}/transcription")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"message":"hello","role":"agent","audio_url":null,"emotion":null}],
                "totalSize": 1
            })))
            .mount(&server)
            .await;
        let t = adapter.get_transcript(&eng).await.expect("ok");
        assert_eq!(t.messages.len(), 1);
        assert_eq!(t.messages[0].message, "hello");
        assert_eq!(t.total_size, 1);
    }

    #[tokio::test]
    async fn get_summary_unwraps_data() {
        let (server, adapter) = make_adapter().await;
        let eng = EngagementId::new();
        Mock::given(method("GET"))
            .and(path(format!("/calls/{eng}/summary")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"summary":"great call","resolution":null,"resolution_explanation":null}
            })))
            .mount(&server)
            .await;
        let s = adapter.get_summary(&eng).await.expect("ok");
        assert_eq!(s.summary, "great call");
    }

    #[tokio::test]
    async fn get_sentiment_unwraps_data() {
        let (server, adapter) = make_adapter().await;
        let eng = EngagementId::new();
        Mock::given(method("GET"))
            .and(path(format!("/calls/{eng}/sentiment")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"sentiment":"positive","justification":"customer was happy"}
            })))
            .mount(&server)
            .await;
        let s = adapter.get_sentiment(&eng).await.expect("ok");
        assert_eq!(s.label, "positive");
    }

    #[tokio::test]
    async fn get_output_extraction_maps_fields() {
        let (server, adapter) = make_adapter().await;
        let eng = EngagementId::new();
        Mock::given(method("GET"))
            .and(path(format!("/calls/{eng}/state")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"key":"name","value":"Alice","audio_file_paths":""}]
            })))
            .mount(&server)
            .await;
        let oe = adapter.get_output_extraction(&eng).await.expect("ok");
        assert_eq!(oe.fields.len(), 1);
        assert_eq!(oe.fields[0].key, "name");
    }

    #[tokio::test]
    async fn list_agent_call_logs_returns_items() {
        let (server, adapter) = make_adapter().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/calls/agent-1/history-call"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id":"c1","created_at":"2026-01-01T00:00:00Z"}],
                "total_size": 1
            })))
            .mount(&server)
            .await;
        let page = adapter
            .list_agent_call_logs(ListAgentCallLogsReq {
                agent_id: "agent-1".into(),
                skip: None,
                limit: None,
                start_date: None,
                end_date: None,
                identity: None,
                id: None,
                batch_id: None,
            })
            .await
            .expect("ok");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.total_size, Some(1));
    }

    #[tokio::test]
    async fn list_org_call_logs_returns_items() {
        let (server, adapter) = make_adapter().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/calls/organizations/org-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id":"c2","created_at":"2026-01-02T00:00:00Z"}],
                "total_size": 1
            })))
            .mount(&server)
            .await;
        let page = adapter
            .list_org_call_logs(ListOrgCallLogsReq {
                org_id: "org-1".into(),
                skip: None,
                limit: None,
                start_date: None,
                end_date: None,
                contact_number: None,
                call_id: None,
            })
            .await
            .expect("ok");
        assert_eq!(page.items.len(), 1);
    }

    #[tokio::test]
    async fn not_found_maps_to_permanent() {
        let (server, adapter) = make_adapter().await;
        let eng = EngagementId::new();
        Mock::given(method("GET"))
            .and(path(format!("/calls/{eng}/transcription")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let err = adapter.get_transcript(&eng).await.expect_err("fail");
        assert!(matches!(err, PostCallError::Permanent(_)));
    }

    #[tokio::test]
    async fn retries_on_503_then_succeeds() {
        let (server, adapter) = make_adapter().await;
        let eng = EngagementId::new();
        Mock::given(method("GET"))
            .and(path(format!("/calls/{eng}/transcription")))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/calls/{eng}/transcription")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [], "totalSize": 0
            })))
            .mount(&server)
            .await;
        let t = adapter.get_transcript(&eng).await.expect("ok after retry");
        assert_eq!(t.messages.len(), 0);
    }
}
