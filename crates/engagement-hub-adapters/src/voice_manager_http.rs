use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::VmError,
    ports::VoiceManagerPort,
    types::{
        CreateTelephonyReq, IssueTestTokenReq, ListTelephoniesReq, StartVoiceSessionReq, StopMode,
        Telephony, TelephonyId, TestToken, UpdateTelephonyReq, VoiceSessionRef,
    },
};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{CLEANUP_RETRY, DEFAULT_RETRY, GRACEFUL_STOP_RETRY, WRITE_RETRY, with_retry},
};

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<ErrorInner>,
}

#[derive(Deserialize)]
struct ErrorInner {
    code: String,
    message: String,
}

fn map_http_status(status: StatusCode, body: &str) -> VmError {
    let detail = serde_json::from_str::<ErrorBody>(body)
        .ok()
        .and_then(|b| b.error)
        .map(|e| format!("{}: {}", e.code, e.message))
        .unwrap_or_else(|| body.to_string());

    match status {
        s if s.is_client_error() => VmError::Permanent(format!("{status}: {detail}")),
        StatusCode::SERVICE_UNAVAILABLE => VmError::Unavailable,
        _ => VmError::Transient(format!("{status}: {detail}")),
    }
}

#[derive(Serialize, Clone)]
struct StartVoiceSessionBody {
    engagement_id: String,
    org_id: String,
}

#[derive(Deserialize)]
struct VoiceSessionRefDto {
    id: String,
}

#[derive(Deserialize)]
struct StartVoiceSessionResp {
    session_ref: VoiceSessionRefDto,
}

#[derive(Serialize, Clone)]
struct IssueTestTokenBody {
    org_id: String,
}

#[derive(Deserialize)]
struct IssueTestTokenResp {
    token: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct TelephonyDto {
    id: String,
    org_id: String,
    phone_number: String,
}

#[derive(Deserialize)]
struct TelephonyResp {
    telephony: TelephonyDto,
}

#[derive(Deserialize)]
struct ListTelephoniesResp {
    telephonies: Vec<TelephonyDto>,
}

#[derive(Serialize, Clone)]
struct CreateTelephonyBody {
    org_id: String,
    phone_number: String,
}

#[derive(Serialize, Clone)]
struct UpdateTelephonyBody {
    phone_number: String,
}

fn telephony_from_dto(t: TelephonyDto) -> Result<Telephony, VmError> {
    let id =
        t.id.parse::<Uuid>()
            .map(TelephonyId::from)
            .map_err(|e| VmError::Permanent(format!("bad telephony id: {e}")))?;
    Ok(Telephony {
        id,
        org_id: t.org_id,
        phone_number: t.phone_number,
    })
}

pub struct VoiceManagerHttpAdapter {
    client: Client,
    base_url: String,
    metrics: Arc<AdapterMetrics>,
}

impl VoiceManagerHttpAdapter {
    pub fn new(client: Client, base_url: String, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client,
            base_url,
            metrics,
        }
    }
}

#[async_trait]
impl VoiceManagerPort for VoiceManagerHttpAdapter {
    async fn start_voice_session(
        &self,
        req: StartVoiceSessionReq,
    ) -> Result<VoiceSessionRef, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/voice/sessions", self.base_url);
        let body = StartVoiceSessionBody {
            engagement_id: req.engagement_id.to_string(),
            org_id: req.org_id.clone(),
        };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            WRITE_RETRY,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let b = body.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .post(&u)
                        .header("X-Request-Id", &rid)
                        .json(&b)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        let parsed: StartVoiceSessionResp = resp
                            .json()
                            .await
                            .map_err(|e| VmError::Permanent(e.to_string()))?;
                        let uid = parsed.session_ref.id.parse::<Uuid>().map_err(|e| {
                            VmError::Permanent(format!("bad session_ref uuid: {e}"))
                        })?;
                        Ok(VoiceSessionRef::new(uid))
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }

    async fn stop_voice_session(
        &self,
        ref_: &VoiceSessionRef,
        mode: StopMode,
    ) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let mode_str = match &mode {
            StopMode::Abort => "abort",
            StopMode::Graceful => "graceful",
        };
        let url = format!(
            "{}/v1/voice/sessions/{}?mode={}",
            self.base_url,
            ref_.as_uuid(),
            mode_str
        );
        let policy = match mode {
            StopMode::Abort => CLEANUP_RETRY,
            StopMode::Graceful => GRACEFUL_STOP_RETRY,
        };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            policy,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .delete(&u)
                        .header("X-Request-Id", &rid)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        Ok(())
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }

    async fn issue_test_token(&self, req: IssueTestTokenReq) -> Result<TestToken, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/voice/test_tokens", self.base_url);
        let body = IssueTestTokenBody {
            org_id: req.org_id.clone(),
        };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            DEFAULT_RETRY,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let b = body.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .post(&u)
                        .header("X-Request-Id", &rid)
                        .json(&b)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        let parsed: IssueTestTokenResp = resp
                            .json()
                            .await
                            .map_err(|e| VmError::Permanent(e.to_string()))?;
                        Ok(TestToken {
                            token: parsed.token,
                        })
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }

    async fn create_telephony(&self, req: CreateTelephonyReq) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/telephonies", self.base_url);
        let body = CreateTelephonyBody {
            org_id: req.org_id.clone(),
            phone_number: req.phone_number.clone(),
        };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            WRITE_RETRY,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let b = body.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .post(&u)
                        .header("X-Request-Id", &rid)
                        .json(&b)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        let parsed: TelephonyResp = resp
                            .json()
                            .await
                            .map_err(|e| VmError::Permanent(e.to_string()))?;
                        telephony_from_dto(parsed.telephony)
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }

    async fn list_telephonies(&self, req: ListTelephoniesReq) -> Result<Vec<Telephony>, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let mut url = format!(
            "{}/v1/telephonies?org_id={}",
            self.base_url,
            urlencoding::encode(&req.org_id)
        );
        if let Some(pt) = &req.page_token {
            url.push_str(&format!("&page={}", urlencoding::encode(pt)));
        }
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            DEFAULT_RETRY,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .get(&u)
                        .header("X-Request-Id", &rid)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        let parsed: ListTelephoniesResp = resp
                            .json()
                            .await
                            .map_err(|e| VmError::Permanent(e.to_string()))?;
                        parsed
                            .telephonies
                            .into_iter()
                            .map(telephony_from_dto)
                            .collect()
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }

    async fn get_telephony(&self, id: &TelephonyId) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/telephonies/{}", self.base_url, id.as_uuid());
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            DEFAULT_RETRY,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .get(&u)
                        .header("X-Request-Id", &rid)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        let parsed: TelephonyResp = resp
                            .json()
                            .await
                            .map_err(|e| VmError::Permanent(e.to_string()))?;
                        telephony_from_dto(parsed.telephony)
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }

    async fn update_telephony(&self, req: UpdateTelephonyReq) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!("{}/v1/telephonies/{}", self.base_url, req.id.as_uuid());
        let body = UpdateTelephonyBody {
            phone_number: req.phone_number.clone(),
        };
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            WRITE_RETRY,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let b = body.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .patch(&u)
                        .header("X-Request-Id", &rid)
                        .json(&b)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        let parsed: TelephonyResp = resp
                            .json()
                            .await
                            .map_err(|e| VmError::Permanent(e.to_string()))?;
                        telephony_from_dto(parsed.telephony)
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }

    async fn delete_telephony(&self, id: &TelephonyId, usage: &str) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let url = format!(
            "{}/v1/telephonies/{}?usage={}",
            self.base_url,
            id.as_uuid(),
            urlencoding::encode(usage)
        );
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        tracing::debug!(adapter.request_id = %request_id, "adapter call");

        with_retry(
            WRITE_RETRY,
            None,
            "voice_manager_http",
            Some(&metrics),
            move || {
                let c = client.clone();
                let u = url.clone();
                let rid = request_id.clone();
                async move {
                    let resp = c
                        .delete(&u)
                        .header("X-Request-Id", &rid)
                        .send()
                        .await
                        .map_err(|e| VmError::Transient(e.to_string()))?;
                    if resp.status().is_success() {
                        Ok(())
                    } else {
                        let s = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        Err(map_http_status(s, &body))
                    }
                }
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engagement_hub_ports::types::EngagementId;
    use serde_json::json;
    use wiremock::matchers::{header_exists, method, path, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn vm(server: &MockServer) -> VoiceManagerHttpAdapter {
        VoiceManagerHttpAdapter::new(Client::new(), server.uri(), AdapterMetrics::for_test())
    }

    #[tokio::test]
    async fn start_voice_session_happy() {
        let server = MockServer::start().await;
        let sid = Uuid::new_v4();
        Mock::given(method("POST"))
            .and(path("/v1/voice/sessions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "session_ref": { "id": sid.to_string() }
            })))
            .mount(&server)
            .await;
        let a = vm(&server).await;
        let r = a
            .start_voice_session(StartVoiceSessionReq {
                engagement_id: EngagementId::default(),
                org_id: "org-1".into(),
            })
            .await
            .expect("ok");
        assert_eq!(r.as_uuid(), &sid);
    }

    #[tokio::test]
    async fn start_voice_session_stamps_x_request_id_header() {
        let server = MockServer::start().await;
        let sid = Uuid::new_v4();
        Mock::given(method("POST"))
            .and(path("/v1/voice/sessions"))
            .and(header_exists("x-request-id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "session_ref": { "id": sid.to_string() }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let a = vm(&server).await;
        a.start_voice_session(StartVoiceSessionReq {
            engagement_id: EngagementId::default(),
            org_id: "org-1".into(),
        })
        .await
        .expect("ok");
    }

    #[tokio::test]
    async fn stop_voice_session_abort_retries_5_times_on_503() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path_regex(r"^/v1/voice/sessions/[0-9a-f-]+$"))
            .and(query_param("mode", "abort"))
            .respond_with(ResponseTemplate::new(503))
            .expect(5)
            .mount(&server)
            .await;
        let a = vm(&server).await;
        let e = a
            .stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Abort)
            .await
            .expect_err("fail");
        assert!(matches!(e, VmError::Unavailable));
    }

    #[tokio::test]
    async fn stop_voice_session_graceful_attempts_only_once_on_503() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path_regex(r"^/v1/voice/sessions/[0-9a-f-]+$"))
            .and(query_param("mode", "graceful"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let a = vm(&server).await;
        let _ = a
            .stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Graceful)
            .await;
    }

    #[tokio::test]
    async fn http_4xx_maps_to_permanent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/voice/sessions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "code": "bad_request", "message": "engagement_id required" }
            })))
            .mount(&server)
            .await;
        let a = vm(&server).await;
        let e = a
            .start_voice_session(StartVoiceSessionReq {
                engagement_id: EngagementId::default(),
                org_id: "org-1".into(),
            })
            .await
            .expect_err("fail");
        assert!(matches!(e, VmError::Permanent(_)));
    }

    #[tokio::test]
    async fn http_500_maps_to_transient_and_retries_for_writes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/telephonies"))
            .respond_with(ResponseTemplate::new(500))
            .expect(2)
            .mount(&server)
            .await;
        let a = vm(&server).await;
        let _ = a
            .create_telephony(CreateTelephonyReq {
                org_id: "org-1".into(),
                phone_number: "+60123456789".into(),
            })
            .await;
    }

    #[tokio::test]
    async fn telephony_crud_happy() {
        let server = MockServer::start().await;
        let tid = Uuid::new_v4();
        let body = json!({
            "telephony": { "id": tid.to_string(), "org_id": "org-1", "phone_number": "+60123456789" }
        });

        Mock::given(method("POST"))
            .and(path("/v1/telephonies"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v1/telephonies/[0-9a-f-]+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path_regex(r"^/v1/telephonies/[0-9a-f-]+$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path_regex(r"^/v1/telephonies/[0-9a-f-]+$"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/telephonies"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"telephonies": []})))
            .mount(&server)
            .await;

        let a = vm(&server).await;
        let t = a
            .create_telephony(CreateTelephonyReq {
                org_id: "org-1".into(),
                phone_number: "+60123456789".into(),
            })
            .await
            .unwrap();
        assert_eq!(t.id, TelephonyId::from(tid));

        let got = a.get_telephony(&TelephonyId::from(tid)).await.unwrap();
        assert_eq!(got.org_id, "org-1");

        let updated = a
            .update_telephony(UpdateTelephonyReq {
                id: TelephonyId::from(tid),
                phone_number: "+60111111111".into(),
            })
            .await
            .unwrap();
        assert_eq!(updated.id, TelephonyId::from(tid));

        a.delete_telephony(&TelephonyId::from(tid), "decommissioned")
            .await
            .unwrap();

        let list = a
            .list_telephonies(ListTelephoniesReq {
                org_id: "org-1".into(),
                page_token: None,
            })
            .await
            .unwrap();
        assert!(list.is_empty());
    }
}
