//! Fake implementation of [`RegistryPort`] for testing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{
    error::RegistryError,
    ports::RegistryPort,
    types::{ResolveSnapshotReq, ResolvedSnapshot, VoiceProfile, VoiceProfileId},
};
use super::Outcome;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct FakeRegistryInner {
    resolve_snapshot: VecDeque<Outcome<ResolvedSnapshot>>,
    get_voice_profile: VecDeque<Outcome<VoiceProfile>>,
}

// ---------------------------------------------------------------------------
// Public fake
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FakeRegistryPort {
    inner: Arc<Mutex<FakeRegistryInner>>,
}

impl FakeRegistryPort {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeRegistryInner {
                resolve_snapshot: VecDeque::new(),
                get_voice_profile: VecDeque::new(),
            })),
        }
    }

    pub fn push_resolve_snapshot(&self, outcome: Outcome<ResolvedSnapshot>) {
        self.inner.lock().unwrap().resolve_snapshot.push_back(outcome);
    }

    pub fn push_get_voice_profile(&self, outcome: Outcome<VoiceProfile>) {
        self.inner.lock().unwrap().get_voice_profile.push_back(outcome);
    }
}

#[async_trait]
impl RegistryPort for FakeRegistryPort {
    async fn resolve_snapshot(
        &self,
        _req: ResolveSnapshotReq,
    ) -> Result<ResolvedSnapshot, RegistryError> {
        match self
            .inner
            .lock()
            .unwrap()
            .resolve_snapshot
            .pop_front()
            .expect("FakeRegistryPort::resolve_snapshot has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(RegistryError::Transient(msg)),
            Outcome::Permanent(msg) => Err(RegistryError::Permanent(msg)),
            Outcome::Unavailable => Err(RegistryError::Unavailable),
            Outcome::Panic => panic!("FakeRegistryPort::resolve_snapshot panic injected"),
        }
    }

    async fn get_voice_profile(
        &self,
        _id: &VoiceProfileId,
    ) -> Result<VoiceProfile, RegistryError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_voice_profile
            .pop_front()
            .expect("FakeRegistryPort::get_voice_profile has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(RegistryError::Transient(msg)),
            Outcome::Permanent(msg) => Err(RegistryError::Permanent(msg)),
            Outcome::Unavailable => Err(RegistryError::Unavailable),
            Outcome::Panic => panic!("FakeRegistryPort::get_voice_profile panic injected"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_snapshot_success() {
        let fake = FakeRegistryPort::new();
        fake.push_resolve_snapshot(Outcome::Success(ResolvedSnapshot {}));
        let result = fake.resolve_snapshot(ResolveSnapshotReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn resolve_snapshot_transient() {
        let fake = FakeRegistryPort::new();
        fake.push_resolve_snapshot(Outcome::Transient("timeout".into()));
        let result = fake.resolve_snapshot(ResolveSnapshotReq {}).await;
        assert!(matches!(result, Err(RegistryError::Transient(_))));
    }

    #[tokio::test]
    async fn resolve_snapshot_permanent() {
        let fake = FakeRegistryPort::new();
        fake.push_resolve_snapshot(Outcome::Permanent("not found".into()));
        let result = fake.resolve_snapshot(ResolveSnapshotReq {}).await;
        assert!(matches!(result, Err(RegistryError::Permanent(_))));
    }

    #[tokio::test]
    async fn resolve_snapshot_unavailable() {
        let fake = FakeRegistryPort::new();
        fake.push_resolve_snapshot(Outcome::Unavailable);
        let result = fake.resolve_snapshot(ResolveSnapshotReq {}).await;
        assert!(matches!(result, Err(RegistryError::Unavailable)));
    }

    #[tokio::test]
    async fn resolve_snapshot_panic() {
        let fake = FakeRegistryPort::new();
        fake.push_resolve_snapshot(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.resolve_snapshot(ResolveSnapshotReq {}).await
        })
        .await;
        assert!(result.is_err()); // JoinError from panic
    }

    #[tokio::test]
    async fn get_voice_profile_success() {
        let fake = FakeRegistryPort::new();
        fake.push_get_voice_profile(Outcome::Success(VoiceProfile {}));
        let id = VoiceProfileId::new();
        let result = fake.get_voice_profile(&id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_voice_profile_transient() {
        let fake = FakeRegistryPort::new();
        fake.push_get_voice_profile(Outcome::Transient("timeout".into()));
        let id = VoiceProfileId::new();
        let result = fake.get_voice_profile(&id).await;
        assert!(matches!(result, Err(RegistryError::Transient(_))));
    }

    #[tokio::test]
    async fn get_voice_profile_permanent() {
        let fake = FakeRegistryPort::new();
        fake.push_get_voice_profile(Outcome::Permanent("not found".into()));
        let id = VoiceProfileId::new();
        let result = fake.get_voice_profile(&id).await;
        assert!(matches!(result, Err(RegistryError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_voice_profile_panic() {
        let fake = FakeRegistryPort::new();
        fake.push_get_voice_profile(Outcome::Panic);
        let id = VoiceProfileId::new();
        let result = tokio::task::spawn(async move {
            fake.get_voice_profile(&id).await
        })
        .await;
        assert!(result.is_err());
    }
}
