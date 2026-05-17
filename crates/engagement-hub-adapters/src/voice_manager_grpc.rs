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
use tonic::{Code, transport::Channel};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{
        CLEANUP_RETRY, DEFAULT_RETRY, RetryConfig, WRITE_RETRY, with_retry,
    },
};

mod proto {
    tonic::include_proto!("revocall.voice.v1");
}
use proto::voice_manager_client::VoiceManagerClient;

fn map_status(s: tonic::Status) -> VmError {
    match s.code() {
        Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::Cancelled => VmError::Permanent(format!("{:?}: {}", s.code(), s.message())),
        Code::Unavailable => VmError::Unavailable,
        _ => VmError::Transient(format!("{:?}: {}", s.code(), s.message())),
    }
}

fn stop_mode_to_proto(m: &StopMode) -> proto::StopMode {
    match m {
        StopMode::Abort => proto::StopMode::Abort,
        StopMode::Graceful => proto::StopMode::Graceful,
    }
}

/// 1 attempt — used for stop_voice_session(mode=Graceful), which is NOT idempotent.
const GRACEFUL_STOP_RETRY: RetryConfig = RetryConfig {
    max_attempts: 1,
    initial_backoff: std::time::Duration::from_millis(50),
    max_backoff: std::time::Duration::from_secs(2),
};

fn telephony_from_proto(t: proto::TelephonyProto) -> Result<Telephony, VmError> {
    let id = t.id.parse::<Uuid>()
        .map(TelephonyId::from)
        .map_err(|e| VmError::Permanent(format!("bad telephony id: {e}")))?;
    Ok(Telephony {
        id,
        org_id: t.org_id,
        phone_number: t.phone_number,
    })
}

pub struct VoiceManagerGrpcAdapter {
    client: VoiceManagerClient<Channel>,
    metrics: Arc<AdapterMetrics>,
}

impl VoiceManagerGrpcAdapter {
    pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client: VoiceManagerClient::new(channel),
            metrics,
        }
    }
}

#[async_trait]
impl VoiceManagerPort for VoiceManagerGrpcAdapter {
    async fn start_voice_session(
        &self,
        req: StartVoiceSessionReq,
    ) -> Result<VoiceSessionRef, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::StartVoiceSessionRequest {
                request_id: request_id.clone(),
                engagement_id: req.engagement_id.to_string(),
                org_id: req.org_id.clone(),
            };
            async move {
                c.start_voice_session(r).await.map_err(map_status).and_then(|resp| {
                    let sr = resp.into_inner().session_ref.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty session_ref".into())
                    })?;
                    let uid = sr.id.parse::<Uuid>().map_err(|e| {
                        VmError::Permanent(format!("bad session_ref uuid: {e}"))
                    })?;
                    Ok(VoiceSessionRef::new(uid))
                })
            }
        })
        .await
    }

    async fn stop_voice_session(
        &self,
        ref_: &VoiceSessionRef,
        mode: StopMode,
    ) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let ref_id = ref_.as_uuid().to_string();
        let mode_proto = stop_mode_to_proto(&mode);
        // Graceful is NOT idempotent — single attempt only. Abort is idempotent — 5 attempts.
        let policy = match mode {
            StopMode::Abort => CLEANUP_RETRY,
            StopMode::Graceful => GRACEFUL_STOP_RETRY,
        };

        with_retry(policy, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::StopVoiceSessionRequest {
                request_id: request_id.clone(),
                session_ref: Some(proto::VoiceSessionRefProto { id: ref_id.clone() }),
                mode: mode_proto as i32,
            };
            async move {
                c.stop_voice_session(r).await.map_err(map_status).map(|_| ())
            }
        })
        .await
    }

    async fn issue_test_token(
        &self,
        req: IssueTestTokenReq,
    ) -> Result<TestToken, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(DEFAULT_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::IssueTestTokenRequest {
                request_id: request_id.clone(),
                org_id: req.org_id.clone(),
            };
            async move {
                c.issue_test_token(r).await.map_err(map_status).map(|resp| TestToken {
                    token: resp.into_inner().token,
                })
            }
        })
        .await
    }

    async fn create_telephony(
        &self,
        req: CreateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::CreateTelephonyRequest {
                request_id: request_id.clone(),
                org_id: req.org_id.clone(),
                phone_number: req.phone_number.clone(),
            };
            async move {
                c.create_telephony(r).await.map_err(map_status).and_then(|resp| {
                    let t = resp.into_inner().telephony.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty telephony".into())
                    })?;
                    telephony_from_proto(t)
                })
            }
        })
        .await
    }

    async fn list_telephonies(
        &self,
        req: ListTelephoniesReq,
    ) -> Result<Vec<Telephony>, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(DEFAULT_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::ListTelephoniesRequest {
                request_id: request_id.clone(),
                org_id: req.org_id.clone(),
                page_token: req.page_token.clone(),
            };
            async move {
                c.list_telephonies(r).await.map_err(map_status).and_then(|resp| {
                    resp.into_inner()
                        .telephonies
                        .into_iter()
                        .map(telephony_from_proto)
                        .collect::<Result<Vec<_>, _>>()
                })
            }
        })
        .await
    }

    async fn get_telephony(
        &self,
        id: &TelephonyId,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let tid = id.as_uuid().to_string();

        with_retry(DEFAULT_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::GetTelephonyRequest {
                request_id: request_id.clone(),
                telephony_id: tid.clone(),
            };
            async move {
                c.get_telephony(r).await.map_err(map_status).and_then(|resp| {
                    let t = resp.into_inner().telephony.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty telephony".into())
                    })?;
                    telephony_from_proto(t)
                })
            }
        })
        .await
    }

    async fn update_telephony(
        &self,
        req: UpdateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let tid = req.id.as_uuid().to_string();

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::UpdateTelephonyRequest {
                request_id: request_id.clone(),
                telephony_id: tid.clone(),
                phone_number: req.phone_number.clone(),
            };
            async move {
                c.update_telephony(r).await.map_err(map_status).and_then(|resp| {
                    let t = resp.into_inner().telephony.ok_or_else(|| {
                        VmError::Permanent("voice_manager: empty telephony".into())
                    })?;
                    telephony_from_proto(t)
                })
            }
        })
        .await
    }

    async fn delete_telephony(
        &self,
        id: &TelephonyId,
        usage: &str,
    ) -> Result<(), VmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let tid = id.as_uuid().to_string();
        let usage = usage.to_string();

        with_retry(WRITE_RETRY, None, "voice_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::DeleteTelephonyRequest {
                request_id: request_id.clone(),
                telephony_id: tid.clone(),
                usage: usage.clone(),
            };
            async move {
                c.delete_telephony(r).await.map_err(map_status).map(|_| ())
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engagement_hub_ports::types::EngagementId;
    use proto::{
        voice_manager_server::{VoiceManager as VmServer, VoiceManagerServer},
        CreateTelephonyRequest, CreateTelephonyResponse, DeleteTelephonyRequest,
        DeleteTelephonyResponse, GetTelephonyRequest, GetTelephonyResponse, IssueTestTokenRequest,
        IssueTestTokenResponse, ListTelephoniesRequest, ListTelephoniesResponse,
        StartVoiceSessionRequest, StartVoiceSessionResponse, StopVoiceSessionRequest,
        StopVoiceSessionResponse, TelephonyProto, UpdateTelephonyRequest, UpdateTelephonyResponse,
        VoiceSessionRefProto,
    };
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, Status, transport::Server};

    #[derive(Default)]
    struct VmCounters {
        start: Mutex<u32>,
        stop: Mutex<u32>,
        token: Mutex<u32>,
        create_tel: Mutex<u32>,
        list_tel: Mutex<u32>,
        get_tel: Mutex<u32>,
        update_tel: Mutex<u32>,
        delete_tel: Mutex<u32>,
    }

    struct MockVm {
        start_result: Mutex<Result<VoiceSessionRefProto, Status>>,
        stop_result: Mutex<Result<(), Status>>,
        token_result: Mutex<Result<String, Status>>,
        telephony_result: Mutex<Result<TelephonyProto, Status>>,
        list_result: Mutex<Result<Vec<TelephonyProto>, Status>>,
        seen_request_ids: Mutex<Vec<String>>,
        seen_stop_modes: Mutex<Vec<i32>>,
        counters: VmCounters,
    }

    impl MockVm {
        fn happy(default_id: Uuid) -> Self {
            Self {
                start_result: Mutex::new(Ok(VoiceSessionRefProto { id: default_id.to_string() })),
                stop_result: Mutex::new(Ok(())),
                token_result: Mutex::new(Ok("token-abc".into())),
                telephony_result: Mutex::new(Ok(TelephonyProto {
                    id: default_id.to_string(),
                    org_id: "org-1".into(),
                    phone_number: "+60123456789".into(),
                })),
                list_result: Mutex::new(Ok(vec![])),
                seen_request_ids: Mutex::new(vec![]),
                seen_stop_modes: Mutex::new(vec![]),
                counters: VmCounters::default(),
            }
        }
    }

    #[tonic::async_trait]
    impl VmServer for MockVm {
        async fn start_voice_session(
            &self,
            req: Request<StartVoiceSessionRequest>,
        ) -> Result<Response<StartVoiceSessionResponse>, Status> {
            *self.counters.start.lock().unwrap() += 1;
            self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
            let r = self.start_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
            Ok(Response::new(StartVoiceSessionResponse { session_ref: Some(r) }))
        }

        async fn stop_voice_session(
            &self,
            req: Request<StopVoiceSessionRequest>,
        ) -> Result<Response<StopVoiceSessionResponse>, Status> {
            *self.counters.stop.lock().unwrap() += 1;
            let inner = req.into_inner();
            self.seen_request_ids.lock().unwrap().push(inner.request_id);
            self.seen_stop_modes.lock().unwrap().push(inner.mode);
            self.stop_result.lock().unwrap().as_ref().map(|_| ()).map_err(|e| e.clone())?;
            Ok(Response::new(StopVoiceSessionResponse {}))
        }

        async fn issue_test_token(
            &self,
            req: Request<IssueTestTokenRequest>,
        ) -> Result<Response<IssueTestTokenResponse>, Status> {
            *self.counters.token.lock().unwrap() += 1;
            self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
            let t = self.token_result.lock().unwrap().as_ref().map(|s| s.clone()).map_err(|e| e.clone())?;
            Ok(Response::new(IssueTestTokenResponse { token: t }))
        }

        async fn create_telephony(
            &self,
            req: Request<CreateTelephonyRequest>,
        ) -> Result<Response<CreateTelephonyResponse>, Status> {
            *self.counters.create_tel.lock().unwrap() += 1;
            self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
            let t = self.telephony_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
            Ok(Response::new(CreateTelephonyResponse { telephony: Some(t) }))
        }

        async fn list_telephonies(
            &self,
            req: Request<ListTelephoniesRequest>,
        ) -> Result<Response<ListTelephoniesResponse>, Status> {
            *self.counters.list_tel.lock().unwrap() += 1;
            self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
            let v = self.list_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
            Ok(Response::new(ListTelephoniesResponse { telephonies: v }))
        }

        async fn get_telephony(
            &self,
            req: Request<GetTelephonyRequest>,
        ) -> Result<Response<GetTelephonyResponse>, Status> {
            *self.counters.get_tel.lock().unwrap() += 1;
            self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
            let t = self.telephony_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
            Ok(Response::new(GetTelephonyResponse { telephony: Some(t) }))
        }

        async fn update_telephony(
            &self,
            req: Request<UpdateTelephonyRequest>,
        ) -> Result<Response<UpdateTelephonyResponse>, Status> {
            *self.counters.update_tel.lock().unwrap() += 1;
            self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
            let t = self.telephony_result.lock().unwrap().as_ref().map(|x| x.clone()).map_err(|e| e.clone())?;
            Ok(Response::new(UpdateTelephonyResponse { telephony: Some(t) }))
        }

        async fn delete_telephony(
            &self,
            req: Request<DeleteTelephonyRequest>,
        ) -> Result<Response<DeleteTelephonyResponse>, Status> {
            *self.counters.delete_tel.lock().unwrap() += 1;
            self.seen_request_ids.lock().unwrap().push(req.into_inner().request_id);
            self.stop_result.lock().unwrap().as_ref().map(|_| ()).map_err(|e| e.clone())?;
            Ok(Response::new(DeleteTelephonyResponse {}))
        }
    }

    async fn start_vm_server(mock: Arc<MockVm>) -> Channel {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(
            Server::builder()
                .add_service(VoiceManagerServer::from_arc(mock))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );
        Channel::from_shared(format!("http://{addr}")).unwrap().connect().await.unwrap()
    }

    #[tokio::test]
    async fn start_voice_session_happy() {
        let sid = Uuid::new_v4();
        let mock = Arc::new(MockVm::happy(sid));
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let r = adapter.start_voice_session(StartVoiceSessionReq {
            engagement_id: EngagementId::default(),
            org_id: "org-1".into(),
        }).await.expect("ok");
        assert_eq!(r.as_uuid(), &sid);
    }

    #[tokio::test]
    async fn stop_voice_session_abort_uses_5_attempts_on_transient() {
        let mock = Arc::new(MockVm {
            stop_result: Mutex::new(Err(Status::unavailable("flaky"))),
            ..MockVm::happy(Uuid::new_v4())
        });
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let _ = adapter.stop_voice_session(
            &VoiceSessionRef::new(Uuid::new_v4()),
            StopMode::Abort,
        ).await;
        assert_eq!(*mock.counters.stop.lock().unwrap(), 5);
    }

    #[tokio::test]
    async fn stop_voice_session_graceful_uses_1_attempt_on_transient() {
        let mock = Arc::new(MockVm {
            stop_result: Mutex::new(Err(Status::unavailable("flaky"))),
            ..MockVm::happy(Uuid::new_v4())
        });
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let _ = adapter.stop_voice_session(
            &VoiceSessionRef::new(Uuid::new_v4()),
            StopMode::Graceful,
        ).await;
        assert_eq!(*mock.counters.stop.lock().unwrap(), 1, "Graceful must not retry");
    }

    #[tokio::test]
    async fn stop_voice_session_passes_mode_correctly() {
        let mock = Arc::new(MockVm::happy(Uuid::new_v4()));
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        adapter.stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Abort).await.unwrap();
        adapter.stop_voice_session(&VoiceSessionRef::new(Uuid::new_v4()), StopMode::Graceful).await.unwrap();
        let modes = mock.seen_stop_modes.lock().unwrap();
        assert_eq!(modes[0], proto::StopMode::Abort as i32);
        assert_eq!(modes[1], proto::StopMode::Graceful as i32);
    }

    #[tokio::test]
    async fn issue_test_token_happy() {
        let mock = Arc::new(MockVm::happy(Uuid::new_v4()));
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock).await,
            AdapterMetrics::for_test(),
        );
        let t = adapter.issue_test_token(IssueTestTokenReq { org_id: "org-1".into() }).await.unwrap();
        assert_eq!(t.token, "token-abc");
    }

    #[tokio::test]
    async fn telephony_crud_roundtrip() {
        let tid = Uuid::new_v4();
        let mock = Arc::new(MockVm::happy(tid));
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let created = adapter.create_telephony(CreateTelephonyReq {
            org_id: "org-1".into(),
            phone_number: "+60123456789".into(),
        }).await.unwrap();
        assert_eq!(created.org_id, "org-1");

        let got = adapter.get_telephony(&TelephonyId::from(tid)).await.unwrap();
        assert_eq!(got.phone_number, "+60123456789");

        let updated = adapter.update_telephony(UpdateTelephonyReq {
            id: TelephonyId::from(tid),
            phone_number: "+60111111111".into(),
        }).await.unwrap();
        assert_eq!(updated.org_id, "org-1");

        adapter.delete_telephony(&TelephonyId::from(tid), "decommissioned").await.unwrap();
        assert_eq!(*mock.counters.delete_tel.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn list_telephonies_passes_page_token() {
        let mock = Arc::new(MockVm::happy(Uuid::new_v4()));
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock).await,
            AdapterMetrics::for_test(),
        );
        let v = adapter.list_telephonies(ListTelephoniesReq {
            org_id: "org-1".into(),
            page_token: Some("next-page".into()),
        }).await.unwrap();
        assert!(v.is_empty());
    }

    #[tokio::test]
    async fn create_telephony_retries_twice_on_transient() {
        let mock = Arc::new(MockVm {
            telephony_result: Mutex::new(Err(Status::unavailable("flaky"))),
            ..MockVm::happy(Uuid::new_v4())
        });
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let _ = adapter.create_telephony(CreateTelephonyReq {
            org_id: "org-1".into(),
            phone_number: "+60123456789".into(),
        }).await;
        assert_eq!(*mock.counters.create_tel.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn issue_test_token_invalid_argument_maps_to_permanent() {
        let mock = Arc::new(MockVm {
            token_result: Mutex::new(Err(Status::invalid_argument("bad org_id"))),
            ..MockVm::happy(Uuid::new_v4())
        });
        let adapter = VoiceManagerGrpcAdapter::new(
            start_vm_server(mock).await,
            AdapterMetrics::for_test(),
        );
        let e = adapter.issue_test_token(IssueTestTokenReq { org_id: "bad".into() })
            .await.expect_err("fail");
        assert!(matches!(e, VmError::Permanent(_)));
    }
}
