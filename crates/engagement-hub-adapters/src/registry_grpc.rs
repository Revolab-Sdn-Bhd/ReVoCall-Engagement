use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::RegistryError,
    ports::RegistryPort,
    types::{ResolveSnapshotReq, ResolvedSnapshot, VoiceProfile, VoiceProfileId},
};
use tonic::{Code, transport::Channel};

use crate::{
    metrics::AdapterMetrics,
    policies::{DEFAULT_RETRY, REGISTRY_RESOLVE_RETRY, with_retry},
};

mod proto {
    tonic::include_proto!("revocall.registry.v1");
}
use proto::registry_client::RegistryClient;

fn map_status(s: tonic::Status) -> RegistryError {
    match s.code() {
        // permanent — business or auth errors, never worth retrying
        Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::Cancelled => RegistryError::Permanent(format!("{:?}: {}", s.code(), s.message())),
        // dedicated unavailable variant (retried by IsRetryable)
        Code::Unavailable => RegistryError::Unavailable,
        // transient — spec says retry on DEADLINE_EXCEEDED, ABORTED, INTERNAL
        // ResourceExhausted and Unknown also get one retry
        _ => RegistryError::Transient(format!("{:?}: {}", s.code(), s.message())),
    }
}

pub struct RegistryGrpcAdapter {
    client: RegistryClient<Channel>,
    metrics: Arc<AdapterMetrics>,
}

impl RegistryGrpcAdapter {
    pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client: RegistryClient::new(channel),
            metrics,
        }
    }
}

#[async_trait]
impl RegistryPort for RegistryGrpcAdapter {
    async fn resolve_snapshot(
        &self,
        req: ResolveSnapshotReq,
    ) -> Result<ResolvedSnapshot, RegistryError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        with_retry(
            REGISTRY_RESOLVE_RETRY,
            "registry",
            Some(&metrics),
            move || {
                let mut c = client.clone();
                let r = proto::ResolveSnapshotRequest {
                    org_id: req.org_id.clone(),
                    journey_version: req.journey_version.clone(),
                };
                async move {
                    c.resolve_snapshot(r)
                        .await
                        .map_err(map_status)
                        .and_then(|resp| {
                            let snap = resp.into_inner().snapshot.ok_or_else(|| {
                                RegistryError::Permanent(
                                    "registry: empty snapshot in response".into(),
                                )
                            })?;
                            Ok(ResolvedSnapshot {
                                snapshot_id: snap.snapshot_id,
                                journey_version: snap.journey_version,
                            })
                        })
                }
            },
        )
        .await
    }

    async fn get_voice_profile(&self, id: &VoiceProfileId) -> Result<VoiceProfile, RegistryError> {
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let id_str = id.as_uuid().to_string();
        with_retry(DEFAULT_RETRY, "registry", Some(&metrics), move || {
            let mut c = client.clone();
            let req = proto::GetVoiceProfileRequest {
                voice_profile_id: id_str.clone(),
            };
            async move {
                c.get_voice_profile(req)
                    .await
                    .map_err(map_status)
                    .and_then(|resp| {
                        let p = resp.into_inner().profile.ok_or_else(|| {
                            RegistryError::Permanent("registry: empty profile in response".into())
                        })?;
                        let uid =
                            p.id.parse::<uuid::Uuid>()
                                .map(VoiceProfileId::from)
                                .map_err(|e| RegistryError::Permanent(format!("bad uuid: {e}")))?;
                        Ok(VoiceProfile {
                            id: uid,
                            name: p.name,
                        })
                    })
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{
        GetVoiceProfileRequest, GetVoiceProfileResponse, ResolveSnapshotRequest,
        ResolveSnapshotResponse, ResolvedSnapshotProto, VoiceProfileProto,
        registry_server::{Registry, RegistryServer},
    };
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response, Status, transport::Server};

    struct MockRegistry {
        snap_result: Mutex<Result<ResolvedSnapshotProto, Status>>,
        profile_result: Mutex<Result<VoiceProfileProto, Status>>,
    }

    #[tonic::async_trait]
    impl Registry for MockRegistry {
        async fn resolve_snapshot(
            &self,
            _req: Request<ResolveSnapshotRequest>,
        ) -> Result<Response<ResolveSnapshotResponse>, Status> {
            let r = self
                .snap_result
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.clone())
                .map_err(|e| e.clone())?;
            Ok(Response::new(ResolveSnapshotResponse { snapshot: Some(r) }))
        }

        async fn get_voice_profile(
            &self,
            _req: Request<GetVoiceProfileRequest>,
        ) -> Result<Response<GetVoiceProfileResponse>, Status> {
            let r = self
                .profile_result
                .lock()
                .unwrap()
                .as_ref()
                .map(|p| p.clone())
                .map_err(|e| e.clone())?;
            Ok(Response::new(GetVoiceProfileResponse { profile: Some(r) }))
        }
    }

    async fn start_server(mock: MockRegistry) -> Channel {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(
            Server::builder()
                .add_service(RegistryServer::new(mock))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );
        Channel::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn resolve_snapshot_success() {
        let mock = MockRegistry {
            snap_result: Mutex::new(Ok(ResolvedSnapshotProto {
                snapshot_id: "snap-grpc".into(),
                journey_version: "v1".into(),
            })),
            profile_result: Mutex::new(Err(Status::not_found("n/a"))),
        };
        let adapter =
            RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
        let snap = adapter
            .resolve_snapshot(ResolveSnapshotReq {
                org_id: "o1".into(),
                journey_version: "v1".into(),
            })
            .await
            .expect("ok");
        assert_eq!(snap.snapshot_id, "snap-grpc");
    }

    #[tokio::test]
    async fn not_found_maps_to_permanent() {
        let mock = MockRegistry {
            snap_result: Mutex::new(Err(Status::not_found("unknown"))),
            profile_result: Mutex::new(Err(Status::not_found("n/a"))),
        };
        let adapter =
            RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
        let err = adapter
            .resolve_snapshot(ResolveSnapshotReq {
                org_id: "o1".into(),
                journey_version: "vX".into(),
            })
            .await
            .expect_err("fail");
        assert!(matches!(err, RegistryError::Permanent(_)));
    }

    #[tokio::test]
    async fn unavailable_maps_correctly() {
        let mock = MockRegistry {
            snap_result: Mutex::new(Err(Status::unavailable("down"))),
            profile_result: Mutex::new(Err(Status::not_found("n/a"))),
        };
        let adapter =
            RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
        let err = adapter
            .resolve_snapshot(ResolveSnapshotReq {
                org_id: "o1".into(),
                journey_version: "v1".into(),
            })
            .await
            .expect_err("fail");
        // After exhausting retries on Unavailable, must map to Unavailable (never Transient)
        assert!(matches!(err, RegistryError::Unavailable));
    }

    #[tokio::test]
    async fn get_voice_profile_success() {
        let profile_id = uuid::Uuid::new_v4();
        let mock = MockRegistry {
            snap_result: Mutex::new(Err(Status::not_found("n/a"))),
            profile_result: Mutex::new(Ok(VoiceProfileProto {
                id: profile_id.to_string(),
                name: "grpc-bot".into(),
            })),
        };
        let adapter =
            RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
        let vp = adapter
            .get_voice_profile(&VoiceProfileId::from(profile_id))
            .await
            .expect("ok");
        assert_eq!(vp.name, "grpc-bot");
    }

    #[tokio::test]
    async fn permission_denied_maps_to_permanent() {
        let mock = MockRegistry {
            snap_result: Mutex::new(Err(Status::permission_denied("not allowed"))),
            profile_result: Mutex::new(Err(Status::not_found("n/a"))),
        };
        let adapter =
            RegistryGrpcAdapter::new(start_server(mock).await, AdapterMetrics::for_test());
        let err = adapter
            .resolve_snapshot(ResolveSnapshotReq {
                org_id: "o1".into(),
                journey_version: "v1".into(),
            })
            .await
            .expect_err("fail");
        assert!(matches!(err, RegistryError::Permanent(_)));
    }
}
