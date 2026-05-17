use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::JmError,
    ports::JourneyManagerPort,
    types::{
        CancelReason, CreateExecutionReq, ExecutionRef, Timeline, TimelineEvent, TimelineOpts,
    },
};
use tonic::{Code, transport::Channel};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{CLEANUP_RETRY, DEFAULT_RETRY, WRITE_RETRY, with_retry},
};

mod proto {
    tonic::include_proto!("revocall.journey.v1");
}
use proto::journey_manager_client::JourneyManagerClient;

fn map_status(s: tonic::Status) -> JmError {
    match s.code() {
        Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::Cancelled => JmError::Permanent(format!("{:?}: {}", s.code(), s.message())),
        Code::Unavailable => JmError::Unavailable,
        _ => JmError::Transient(format!("{:?}: {}", s.code(), s.message())),
    }
}

fn cancel_reason_to_proto(r: CancelReason) -> proto::CancelReason {
    match r {
        CancelReason::CompensateFailedBind => proto::CancelReason::CompensateFailedBind,
        CancelReason::UserRequested => proto::CancelReason::UserRequested,
        CancelReason::OrchestratorTimeout => proto::CancelReason::OrchestratorTimeout,
        CancelReason::AdminCancelled => proto::CancelReason::AdminCancelled,
    }
}

pub struct JourneyManagerGrpcAdapter {
    client: JourneyManagerClient<Channel>,
    metrics: Arc<AdapterMetrics>,
}

impl JourneyManagerGrpcAdapter {
    pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client: JourneyManagerClient::new(channel),
            metrics,
        }
    }
}

#[async_trait]
impl JourneyManagerPort for JourneyManagerGrpcAdapter {
    async fn create_execution(
        &self,
        req: CreateExecutionReq,
    ) -> Result<ExecutionRef, JmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());

        with_retry(WRITE_RETRY, None, "journey_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::CreateExecutionRequest {
                request_id: request_id.clone(),
                journey_version: req.journey_version.clone(),
                org_id: req.org_id.clone(),
                engagement_id: req.engagement_id.to_string(),
            };
            async move {
                c.create_execution(r)
                    .await
                    .map_err(map_status)
                    .and_then(|resp| {
                        let er = resp
                            .into_inner()
                            .execution_ref
                            .ok_or_else(|| {
                                JmError::Permanent("journey_manager: empty execution_ref".into())
                            })?;
                        let uid = er.id.parse::<Uuid>().map_err(|e| {
                            JmError::Permanent(format!("bad execution_ref uuid: {e}"))
                        })?;
                        Ok(ExecutionRef::new(uid))
                    })
            }
        })
        .await
    }

    async fn cancel_execution(
        &self,
        ref_: &ExecutionRef,
        reason: CancelReason,
    ) -> Result<(), JmError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let request_id = Uuid::new_v4().to_string();
        tracing::Span::current().record("adapter.request_id", request_id.as_str());
        let ref_id = ref_.as_uuid().to_string();
        let reason_proto = cancel_reason_to_proto(reason);

        with_retry(CLEANUP_RETRY, None, "journey_manager", Some(&metrics), move || {
            let mut c = client.clone();
            let r = proto::CancelExecutionRequest {
                request_id: request_id.clone(),
                execution_ref: Some(proto::ExecutionRefProto { id: ref_id.clone() }),
                reason: reason_proto as i32,
            };
            async move {
                c.cancel_execution(r).await.map_err(map_status).map(|_| ())
            }
        })
        .await
    }

    async fn get_execution_timeline(
        &self,
        _ref_: &ExecutionRef,
        _opts: TimelineOpts,
    ) -> Result<Timeline, JmError> {
        // Implemented in Task 12.
        unimplemented!("see Task 12")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engagement_hub_ports::types::EngagementId;
    use proto::{
        journey_manager_server::{JourneyManager as JmServer, JourneyManagerServer},
        CancelExecutionRequest, CancelExecutionResponse, CreateExecutionRequest,
        CreateExecutionResponse, ExecutionRefProto, GetExecutionTimelineRequest,
        GetExecutionTimelineResponse,
    };
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, Status, transport::Server};

    #[derive(Default)]
    struct CallCounters {
        create: Mutex<u32>,
        cancel: Mutex<u32>,
        timeline: Mutex<u32>,
    }

    struct MockJm {
        create_result: Mutex<Result<ExecutionRefProto, Status>>,
        cancel_result: Mutex<Result<(), Status>>,
        timeline_result: Mutex<Result<Vec<proto::TimelineEventProto>, Status>>,
        seen_request_ids: Mutex<Vec<String>>,
        counters: CallCounters,
    }

    impl MockJm {
        fn always_ok_create(ref_id: Uuid) -> Self {
            Self {
                create_result: Mutex::new(Ok(ExecutionRefProto {
                    id: ref_id.to_string(),
                })),
                cancel_result: Mutex::new(Ok(())),
                timeline_result: Mutex::new(Ok(vec![])),
                seen_request_ids: Mutex::new(vec![]),
                counters: CallCounters::default(),
            }
        }
    }

    #[tonic::async_trait]
    impl JmServer for MockJm {
        async fn create_execution(
            &self,
            req: Request<CreateExecutionRequest>,
        ) -> Result<Response<CreateExecutionResponse>, Status> {
            *self.counters.create.lock().unwrap() += 1;
            self.seen_request_ids
                .lock()
                .unwrap()
                .push(req.into_inner().request_id);
            let r = self
                .create_result
                .lock()
                .unwrap()
                .as_ref()
                .map(|x| x.clone())
                .map_err(|e| e.clone())?;
            Ok(Response::new(CreateExecutionResponse {
                execution_ref: Some(r),
            }))
        }

        async fn cancel_execution(
            &self,
            req: Request<CancelExecutionRequest>,
        ) -> Result<Response<CancelExecutionResponse>, Status> {
            *self.counters.cancel.lock().unwrap() += 1;
            self.seen_request_ids
                .lock()
                .unwrap()
                .push(req.into_inner().request_id);
            self.cancel_result
                .lock()
                .unwrap()
                .as_ref()
                .map(|_| ())
                .map_err(|e| e.clone())?;
            Ok(Response::new(CancelExecutionResponse {}))
        }

        async fn get_execution_timeline(
            &self,
            req: Request<GetExecutionTimelineRequest>,
        ) -> Result<Response<GetExecutionTimelineResponse>, Status> {
            *self.counters.timeline.lock().unwrap() += 1;
            self.seen_request_ids
                .lock()
                .unwrap()
                .push(req.into_inner().request_id);
            let events = self
                .timeline_result
                .lock()
                .unwrap()
                .as_ref()
                .map(|v| v.clone())
                .map_err(|e| e.clone())?;
            Ok(Response::new(GetExecutionTimelineResponse { events }))
        }
    }

    async fn start_server(mock: Arc<MockJm>) -> Channel {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(
            Server::builder()
                .add_service(JourneyManagerServer::from_arc(mock))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );
        Channel::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_execution_happy_path() {
        let exec_id = Uuid::new_v4();
        let mock = Arc::new(MockJm::always_ok_create(exec_id));
        let adapter = JourneyManagerGrpcAdapter::new(
            start_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let r = adapter
            .create_execution(CreateExecutionReq {
                journey_version: "v1".into(),
                org_id: "org-1".into(),
                engagement_id: EngagementId::default(),
            })
            .await
            .expect("ok");
        assert_eq!(r.as_uuid(), &exec_id);
        // request_id was stamped (non-empty UUID string).
        let ids = mock.seen_request_ids.lock().unwrap();
        assert_eq!(ids.len(), 1);
        Uuid::parse_str(&ids[0]).expect("stamped request_id is a UUID");
    }

    #[tokio::test]
    async fn create_execution_invalid_argument_maps_to_permanent() {
        let mock = Arc::new(MockJm {
            create_result: Mutex::new(Err(Status::invalid_argument("bad journey_version"))),
            cancel_result: Mutex::new(Ok(())),
            timeline_result: Mutex::new(Ok(vec![])),
            seen_request_ids: Mutex::new(vec![]),
            counters: CallCounters::default(),
        });
        let adapter = JourneyManagerGrpcAdapter::new(
            start_server(mock).await,
            AdapterMetrics::for_test(),
        );
        let err = adapter
            .create_execution(CreateExecutionReq {
                journey_version: "bogus".into(),
                org_id: "org-1".into(),
                engagement_id: EngagementId::default(),
            })
            .await
            .expect_err("fail");
        assert!(matches!(err, JmError::Permanent(_)));
    }

    #[tokio::test]
    async fn create_execution_unavailable_maps_to_unavailable() {
        let mock = Arc::new(MockJm {
            create_result: Mutex::new(Err(Status::unavailable("down"))),
            cancel_result: Mutex::new(Ok(())),
            timeline_result: Mutex::new(Ok(vec![])),
            seen_request_ids: Mutex::new(vec![]),
            counters: CallCounters::default(),
        });
        let adapter = JourneyManagerGrpcAdapter::new(
            start_server(mock).await,
            AdapterMetrics::for_test(),
        );
        let err = adapter
            .create_execution(CreateExecutionReq {
                journey_version: "v1".into(),
                org_id: "org-1".into(),
                engagement_id: EngagementId::default(),
            })
            .await
            .expect_err("fail");
        assert!(matches!(err, JmError::Unavailable));
    }

    #[tokio::test]
    async fn create_execution_retries_exactly_twice_on_transient() {
        let mock = Arc::new(MockJm {
            create_result: Mutex::new(Err(Status::unavailable("flaky"))),
            cancel_result: Mutex::new(Ok(())),
            timeline_result: Mutex::new(Ok(vec![])),
            seen_request_ids: Mutex::new(vec![]),
            counters: CallCounters::default(),
        });
        let adapter = JourneyManagerGrpcAdapter::new(
            start_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let _ = adapter
            .create_execution(CreateExecutionReq {
                journey_version: "v1".into(),
                org_id: "org-1".into(),
                engagement_id: EngagementId::default(),
            })
            .await;
        assert_eq!(*mock.counters.create.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn create_execution_request_id_is_stable_across_retries() {
        let mock = Arc::new(MockJm {
            create_result: Mutex::new(Err(Status::unavailable("flaky"))),
            cancel_result: Mutex::new(Ok(())),
            timeline_result: Mutex::new(Ok(vec![])),
            seen_request_ids: Mutex::new(vec![]),
            counters: CallCounters::default(),
        });
        let adapter = JourneyManagerGrpcAdapter::new(
            start_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let _ = adapter
            .create_execution(CreateExecutionReq {
                journey_version: "v1".into(),
                org_id: "org-1".into(),
                engagement_id: EngagementId::default(),
            })
            .await;
        let ids = mock.seen_request_ids.lock().unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ids[1], "request_id must be stable across retries");
    }

    // ── cancel_execution tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_execution_happy_path() {
        let mock = Arc::new(MockJm::always_ok_create(Uuid::new_v4()));
        let adapter = JourneyManagerGrpcAdapter::new(
            start_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        adapter
            .cancel_execution(
                &ExecutionRef::new(Uuid::new_v4()),
                CancelReason::CompensateFailedBind,
            )
            .await
            .expect("ok");
        assert_eq!(*mock.counters.cancel.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn cancel_execution_retries_five_times_on_transient() {
        let mock = Arc::new(MockJm {
            create_result: Mutex::new(Ok(ExecutionRefProto { id: Uuid::new_v4().to_string() })),
            cancel_result: Mutex::new(Err(Status::unavailable("flaky"))),
            timeline_result: Mutex::new(Ok(vec![])),
            seen_request_ids: Mutex::new(vec![]),
            counters: CallCounters::default(),
        });
        let adapter = JourneyManagerGrpcAdapter::new(
            start_server(mock.clone()).await,
            AdapterMetrics::for_test(),
        );
        let _ = adapter
            .cancel_execution(
                &ExecutionRef::new(Uuid::new_v4()),
                CancelReason::CompensateFailedBind,
            )
            .await;
        assert_eq!(*mock.counters.cancel.lock().unwrap(), 5);
    }
}
