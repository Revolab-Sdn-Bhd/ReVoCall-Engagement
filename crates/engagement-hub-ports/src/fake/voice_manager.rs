//! Fake implementation of [`VoiceManagerPort`] for testing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{
    error::VmError,
    ports::VoiceManagerPort,
    types::{
        CreateTelephonyReq, IssueTestTokenReq, ListTelephoniesReq, StartVoiceSessionReq,
        StopMode, Telephony, TelephonyId, TestToken, UpdateTelephonyReq, VoiceSessionRef,
    },
};
use super::Outcome;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct FakeVoiceManagerInner {
    start_voice_session: VecDeque<Outcome<VoiceSessionRef>>,
    stop_voice_session: VecDeque<Outcome<()>>,
    issue_test_token: VecDeque<Outcome<TestToken>>,
    create_telephony: VecDeque<Outcome<Telephony>>,
    list_telephonies: VecDeque<Outcome<Vec<Telephony>>>,
    get_telephony: VecDeque<Outcome<Telephony>>,
    update_telephony: VecDeque<Outcome<Telephony>>,
    delete_telephony: VecDeque<Outcome<()>>,
}

// ---------------------------------------------------------------------------
// Public fake
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FakeVoiceManagerPort {
    inner: Arc<Mutex<FakeVoiceManagerInner>>,
}

impl FakeVoiceManagerPort {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeVoiceManagerInner {
                start_voice_session: VecDeque::new(),
                stop_voice_session: VecDeque::new(),
                issue_test_token: VecDeque::new(),
                create_telephony: VecDeque::new(),
                list_telephonies: VecDeque::new(),
                get_telephony: VecDeque::new(),
                update_telephony: VecDeque::new(),
                delete_telephony: VecDeque::new(),
            })),
        }
    }

    pub fn push_start_voice_session(&self, outcome: Outcome<VoiceSessionRef>) {
        self.inner.lock().unwrap().start_voice_session.push_back(outcome);
    }

    pub fn push_stop_voice_session(&self, outcome: Outcome<()>) {
        self.inner.lock().unwrap().stop_voice_session.push_back(outcome);
    }

    pub fn push_issue_test_token(&self, outcome: Outcome<TestToken>) {
        self.inner.lock().unwrap().issue_test_token.push_back(outcome);
    }

    pub fn push_create_telephony(&self, outcome: Outcome<Telephony>) {
        self.inner.lock().unwrap().create_telephony.push_back(outcome);
    }

    pub fn push_list_telephonies(&self, outcome: Outcome<Vec<Telephony>>) {
        self.inner.lock().unwrap().list_telephonies.push_back(outcome);
    }

    pub fn push_get_telephony(&self, outcome: Outcome<Telephony>) {
        self.inner.lock().unwrap().get_telephony.push_back(outcome);
    }

    pub fn push_update_telephony(&self, outcome: Outcome<Telephony>) {
        self.inner.lock().unwrap().update_telephony.push_back(outcome);
    }

    pub fn push_delete_telephony(&self, outcome: Outcome<()>) {
        self.inner.lock().unwrap().delete_telephony.push_back(outcome);
    }
}

#[async_trait]
impl VoiceManagerPort for FakeVoiceManagerPort {
    async fn start_voice_session(
        &self,
        _req: StartVoiceSessionReq,
    ) -> Result<VoiceSessionRef, VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .start_voice_session
            .pop_front()
            .expect("FakeVoiceManagerPort::start_voice_session has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::start_voice_session panic injected"),
        }
    }

    async fn stop_voice_session(
        &self,
        _ref_: &VoiceSessionRef,
        _mode: StopMode,
    ) -> Result<(), VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .stop_voice_session
            .pop_front()
            .expect("FakeVoiceManagerPort::stop_voice_session has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::stop_voice_session panic injected"),
        }
    }

    async fn issue_test_token(
        &self,
        _req: IssueTestTokenReq,
    ) -> Result<TestToken, VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .issue_test_token
            .pop_front()
            .expect("FakeVoiceManagerPort::issue_test_token has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::issue_test_token panic injected"),
        }
    }

    async fn create_telephony(
        &self,
        _req: CreateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .create_telephony
            .pop_front()
            .expect("FakeVoiceManagerPort::create_telephony has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::create_telephony panic injected"),
        }
    }

    async fn list_telephonies(
        &self,
        _req: ListTelephoniesReq,
    ) -> Result<Vec<Telephony>, VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .list_telephonies
            .pop_front()
            .expect("FakeVoiceManagerPort::list_telephonies has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::list_telephonies panic injected"),
        }
    }

    async fn get_telephony(
        &self,
        _id: &TelephonyId,
    ) -> Result<Telephony, VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_telephony
            .pop_front()
            .expect("FakeVoiceManagerPort::get_telephony has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::get_telephony panic injected"),
        }
    }

    async fn update_telephony(
        &self,
        _req: UpdateTelephonyReq,
    ) -> Result<Telephony, VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .update_telephony
            .pop_front()
            .expect("FakeVoiceManagerPort::update_telephony has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::update_telephony panic injected"),
        }
    }

    async fn delete_telephony(
        &self,
        _id: &TelephonyId,
        _usage: &str,
    ) -> Result<(), VmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .delete_telephony
            .pop_front()
            .expect("FakeVoiceManagerPort::delete_telephony has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(VmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(VmError::Permanent(msg)),
            Outcome::Unavailable => Err(VmError::Unavailable),
            Outcome::Panic => panic!("FakeVoiceManagerPort::delete_telephony panic injected"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn session_ref() -> VoiceSessionRef {
        VoiceSessionRef { id: Uuid::new_v4() }
    }

    fn telephony() -> Telephony {
        Telephony { id: TelephonyId::new() }
    }

    // --- start_voice_session ---

    #[tokio::test]
    async fn start_voice_session_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_start_voice_session(Outcome::Success(session_ref()));
        let result = fake.start_voice_session(StartVoiceSessionReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn start_voice_session_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_start_voice_session(Outcome::Transient("timeout".into()));
        let result = fake.start_voice_session(StartVoiceSessionReq {}).await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn start_voice_session_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_start_voice_session(Outcome::Permanent("invalid".into()));
        let result = fake.start_voice_session(StartVoiceSessionReq {}).await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn start_voice_session_unavailable() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_start_voice_session(Outcome::Unavailable);
        let result = fake.start_voice_session(StartVoiceSessionReq {}).await;
        assert!(matches!(result, Err(VmError::Unavailable)));
    }

    #[tokio::test]
    async fn start_voice_session_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_start_voice_session(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.start_voice_session(StartVoiceSessionReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- stop_voice_session ---

    #[tokio::test]
    async fn stop_voice_session_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_stop_voice_session(Outcome::Success(()));
        let ref_ = session_ref();
        let result = fake.stop_voice_session(&ref_, StopMode::Graceful).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn stop_voice_session_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_stop_voice_session(Outcome::Transient("timeout".into()));
        let ref_ = session_ref();
        let result = fake.stop_voice_session(&ref_, StopMode::Graceful).await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn stop_voice_session_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_stop_voice_session(Outcome::Permanent("not found".into()));
        let ref_ = session_ref();
        let result = fake.stop_voice_session(&ref_, StopMode::Graceful).await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn stop_voice_session_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_stop_voice_session(Outcome::Panic);
        let ref_ = session_ref();
        let result = tokio::task::spawn(async move {
            fake.stop_voice_session(&ref_, StopMode::Graceful).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- issue_test_token ---

    #[tokio::test]
    async fn issue_test_token_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_issue_test_token(Outcome::Success(TestToken { token: "tok".into() }));
        let result = fake.issue_test_token(IssueTestTokenReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn issue_test_token_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_issue_test_token(Outcome::Transient("timeout".into()));
        let result = fake.issue_test_token(IssueTestTokenReq {}).await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn issue_test_token_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_issue_test_token(Outcome::Permanent("denied".into()));
        let result = fake.issue_test_token(IssueTestTokenReq {}).await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn issue_test_token_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_issue_test_token(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.issue_test_token(IssueTestTokenReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- create_telephony ---

    #[tokio::test]
    async fn create_telephony_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_create_telephony(Outcome::Success(telephony()));
        let result = fake.create_telephony(CreateTelephonyReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_telephony_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_create_telephony(Outcome::Transient("timeout".into()));
        let result = fake.create_telephony(CreateTelephonyReq {}).await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn create_telephony_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_create_telephony(Outcome::Permanent("invalid".into()));
        let result = fake.create_telephony(CreateTelephonyReq {}).await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn create_telephony_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_create_telephony(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.create_telephony(CreateTelephonyReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- list_telephonies ---

    #[tokio::test]
    async fn list_telephonies_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_list_telephonies(Outcome::Success(vec![telephony()]));
        let result = fake.list_telephonies(ListTelephoniesReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn list_telephonies_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_list_telephonies(Outcome::Transient("timeout".into()));
        let result = fake.list_telephonies(ListTelephoniesReq {}).await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn list_telephonies_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_list_telephonies(Outcome::Permanent("forbidden".into()));
        let result = fake.list_telephonies(ListTelephoniesReq {}).await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn list_telephonies_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_list_telephonies(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.list_telephonies(ListTelephoniesReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- get_telephony ---

    #[tokio::test]
    async fn get_telephony_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_get_telephony(Outcome::Success(telephony()));
        let id = TelephonyId::new();
        let result = fake.get_telephony(&id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_telephony_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_get_telephony(Outcome::Transient("timeout".into()));
        let id = TelephonyId::new();
        let result = fake.get_telephony(&id).await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn get_telephony_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_get_telephony(Outcome::Permanent("not found".into()));
        let id = TelephonyId::new();
        let result = fake.get_telephony(&id).await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_telephony_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_get_telephony(Outcome::Panic);
        let id = TelephonyId::new();
        let result = tokio::task::spawn(async move {
            fake.get_telephony(&id).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- update_telephony ---

    #[tokio::test]
    async fn update_telephony_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_update_telephony(Outcome::Success(telephony()));
        let result = fake.update_telephony(UpdateTelephonyReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn update_telephony_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_update_telephony(Outcome::Transient("timeout".into()));
        let result = fake.update_telephony(UpdateTelephonyReq {}).await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn update_telephony_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_update_telephony(Outcome::Permanent("conflict".into()));
        let result = fake.update_telephony(UpdateTelephonyReq {}).await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn update_telephony_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_update_telephony(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.update_telephony(UpdateTelephonyReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- delete_telephony ---

    #[tokio::test]
    async fn delete_telephony_success() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_delete_telephony(Outcome::Success(()));
        let id = TelephonyId::new();
        let result = fake.delete_telephony(&id, "none").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn delete_telephony_transient() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_delete_telephony(Outcome::Transient("timeout".into()));
        let id = TelephonyId::new();
        let result = fake.delete_telephony(&id, "none").await;
        assert!(matches!(result, Err(VmError::Transient(_))));
    }

    #[tokio::test]
    async fn delete_telephony_permanent() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_delete_telephony(Outcome::Permanent("not found".into()));
        let id = TelephonyId::new();
        let result = fake.delete_telephony(&id, "none").await;
        assert!(matches!(result, Err(VmError::Permanent(_))));
    }

    #[tokio::test]
    async fn delete_telephony_panic() {
        let fake = FakeVoiceManagerPort::new();
        fake.push_delete_telephony(Outcome::Panic);
        let id = TelephonyId::new();
        let result = tokio::task::spawn(async move {
            fake.delete_telephony(&id, "none").await
        })
        .await;
        assert!(result.is_err());
    }
}
