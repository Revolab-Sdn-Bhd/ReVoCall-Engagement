use std::collections::HashMap;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::RegistryError,
    ports::RegistryPort,
    types::{ResolveSnapshotReq, ResolvedSnapshot, VoiceProfile, VoiceProfileId},
};

pub struct RegistryStubAdapter {
    snapshots: HashMap<String, ResolvedSnapshot>,
    profiles: HashMap<String, VoiceProfile>,
}

impl RegistryStubAdapter {
    /// Build from explicit fixture lists. Snapshot map key = `journey_version`.
    /// Profile map key = UUID string of the `VoiceProfileId`.
    pub fn new(snapshots: Vec<ResolvedSnapshot>, profiles: Vec<VoiceProfile>) -> Self {
        Self {
            snapshots: snapshots.into_iter().map(|s| (s.journey_version.clone(), s)).collect(),
            profiles: profiles.into_iter().map(|p| (p.id.as_uuid().to_string(), p)).collect(),
        }
    }

    /// Default fixtures for Track 0 idle-mode deployments.
    pub fn with_default_fixtures() -> Self {
        Self::new(
            vec![ResolvedSnapshot {
                snapshot_id: "fixture-snap-v1".into(),
                journey_version: "v1-fixture".into(),
            }],
            vec![],
        )
    }
}

#[async_trait]
impl RegistryPort for RegistryStubAdapter {
    async fn resolve_snapshot(
        &self,
        req: ResolveSnapshotReq,
    ) -> Result<ResolvedSnapshot, RegistryError> {
        self.snapshots
            .get(&req.journey_version)
            .cloned()
            .ok_or_else(|| RegistryError::Permanent(
                format!("stub: journey_version '{}' not found in fixtures", req.journey_version)
            ))
    }

    async fn get_voice_profile(
        &self,
        id: &VoiceProfileId,
    ) -> Result<VoiceProfile, RegistryError> {
        self.profiles
            .get(&id.as_uuid().to_string())
            .cloned()
            .ok_or_else(|| RegistryError::Permanent(
                format!("stub: voice_profile_id '{}' not found in fixtures", id)
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn stub() -> RegistryStubAdapter {
        RegistryStubAdapter::new(
            vec![
                ResolvedSnapshot { snapshot_id: "snap-1".into(), journey_version: "v1".into() },
                ResolvedSnapshot { snapshot_id: "snap-2".into(), journey_version: "v2".into() },
            ],
            vec![
                VoiceProfile { id: VoiceProfileId::from(Uuid::nil()), name: "bot".into() },
            ],
        )
    }

    #[tokio::test]
    async fn resolve_known_version() {
        let snap = stub().resolve_snapshot(ResolveSnapshotReq {
            org_id: "org1".into(), journey_version: "v1".into(),
        }).await.expect("found");
        assert_eq!(snap.snapshot_id, "snap-1");
    }

    #[tokio::test]
    async fn resolve_unknown_version_returns_permanent() {
        let err = stub().resolve_snapshot(ResolveSnapshotReq {
            org_id: "org1".into(), journey_version: "vX".into(),
        }).await.expect_err("not found");
        assert!(matches!(err, RegistryError::Permanent(_)));
    }

    #[tokio::test]
    async fn get_known_profile() {
        let vp = stub().get_voice_profile(&VoiceProfileId::from(Uuid::nil())).await.expect("found");
        assert_eq!(vp.name, "bot");
    }

    #[tokio::test]
    async fn get_unknown_profile_returns_permanent() {
        let err = stub().get_voice_profile(&VoiceProfileId::new()).await.expect_err("not found");
        assert!(matches!(err, RegistryError::Permanent(_)));
    }

    #[tokio::test]
    async fn default_fixtures_resolves_v1_fixture() {
        let s = RegistryStubAdapter::with_default_fixtures();
        let snap = s.resolve_snapshot(ResolveSnapshotReq {
            org_id: "org1".into(), journey_version: "v1-fixture".into(),
        }).await.expect("default fixture");
        assert_eq!(snap.snapshot_id, "fixture-snap-v1");
    }
}
