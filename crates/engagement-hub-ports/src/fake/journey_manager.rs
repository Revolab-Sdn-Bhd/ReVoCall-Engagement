//! Fake implementation of [`JourneyManagerPort`] for testing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{
    error::JmError,
    ports::JourneyManagerPort,
    types::{
        CancelReason, CreateExecutionReq, ExecutionRef, Timeline, TimelineOpts,
    },
};
use super::Outcome;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct FakeJourneyManagerInner {
    create_execution: VecDeque<Outcome<ExecutionRef>>,
    cancel_execution: VecDeque<Outcome<()>>,
    get_execution_timeline: VecDeque<Outcome<Timeline>>,
}

// ---------------------------------------------------------------------------
// Public fake
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FakeJourneyManagerPort {
    inner: Arc<Mutex<FakeJourneyManagerInner>>,
}

impl FakeJourneyManagerPort {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeJourneyManagerInner {
                create_execution: VecDeque::new(),
                cancel_execution: VecDeque::new(),
                get_execution_timeline: VecDeque::new(),
            })),
        }
    }

    pub fn push_create_execution(&self, outcome: Outcome<ExecutionRef>) {
        self.inner.lock().unwrap().create_execution.push_back(outcome);
    }

    pub fn push_cancel_execution(&self, outcome: Outcome<()>) {
        self.inner.lock().unwrap().cancel_execution.push_back(outcome);
    }

    pub fn push_get_execution_timeline(&self, outcome: Outcome<Timeline>) {
        self.inner.lock().unwrap().get_execution_timeline.push_back(outcome);
    }
}

#[async_trait]
impl JourneyManagerPort for FakeJourneyManagerPort {
    async fn create_execution(
        &self,
        _req: CreateExecutionReq,
    ) -> Result<ExecutionRef, JmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .create_execution
            .pop_front()
            .expect("FakeJourneyManagerPort::create_execution has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(JmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(JmError::Permanent(msg)),
            Outcome::Panic => panic!("FakeJourneyManagerPort::create_execution panic injected"),
        }
    }

    async fn cancel_execution(
        &self,
        _ref_: &ExecutionRef,
        _reason: CancelReason,
    ) -> Result<(), JmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .cancel_execution
            .pop_front()
            .expect("FakeJourneyManagerPort::cancel_execution has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(JmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(JmError::Permanent(msg)),
            Outcome::Panic => panic!("FakeJourneyManagerPort::cancel_execution panic injected"),
        }
    }

    async fn get_execution_timeline(
        &self,
        _ref_: &ExecutionRef,
        _opts: TimelineOpts,
    ) -> Result<Timeline, JmError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_execution_timeline
            .pop_front()
            .expect("FakeJourneyManagerPort::get_execution_timeline has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(JmError::Transient(msg)),
            Outcome::Permanent(msg) => Err(JmError::Permanent(msg)),
            Outcome::Panic => {
                panic!("FakeJourneyManagerPort::get_execution_timeline panic injected")
            }
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

    fn exec_ref() -> ExecutionRef {
        ExecutionRef { id: Uuid::new_v4() }
    }

    #[tokio::test]
    async fn create_execution_success() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_create_execution(Outcome::Success(exec_ref()));
        let result = fake.create_execution(CreateExecutionReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_execution_transient() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_create_execution(Outcome::Transient("timeout".into()));
        let result = fake.create_execution(CreateExecutionReq {}).await;
        assert!(matches!(result, Err(JmError::Transient(_))));
    }

    #[tokio::test]
    async fn create_execution_permanent() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_create_execution(Outcome::Permanent("invalid".into()));
        let result = fake.create_execution(CreateExecutionReq {}).await;
        assert!(matches!(result, Err(JmError::Permanent(_))));
    }

    #[tokio::test]
    async fn create_execution_panic() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_create_execution(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.create_execution(CreateExecutionReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cancel_execution_success() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_cancel_execution(Outcome::Success(()));
        let ref_ = exec_ref();
        let result = fake.cancel_execution(&ref_, CancelReason::UserRequested).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cancel_execution_transient() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_cancel_execution(Outcome::Transient("timeout".into()));
        let ref_ = exec_ref();
        let result = fake.cancel_execution(&ref_, CancelReason::UserRequested).await;
        assert!(matches!(result, Err(JmError::Transient(_))));
    }

    #[tokio::test]
    async fn cancel_execution_permanent() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_cancel_execution(Outcome::Permanent("not found".into()));
        let ref_ = exec_ref();
        let result = fake.cancel_execution(&ref_, CancelReason::UserRequested).await;
        assert!(matches!(result, Err(JmError::Permanent(_))));
    }

    #[tokio::test]
    async fn cancel_execution_panic() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_cancel_execution(Outcome::Panic);
        let ref_ = exec_ref();
        let result = tokio::task::spawn(async move {
            fake.cancel_execution(&ref_, CancelReason::UserRequested).await
        })
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_execution_timeline_success() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_get_execution_timeline(Outcome::Success(Timeline {}));
        let ref_ = exec_ref();
        let result = fake.get_execution_timeline(&ref_, TimelineOpts {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_execution_timeline_transient() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_get_execution_timeline(Outcome::Transient("timeout".into()));
        let ref_ = exec_ref();
        let result = fake.get_execution_timeline(&ref_, TimelineOpts {}).await;
        assert!(matches!(result, Err(JmError::Transient(_))));
    }

    #[tokio::test]
    async fn get_execution_timeline_permanent() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_get_execution_timeline(Outcome::Permanent("not found".into()));
        let ref_ = exec_ref();
        let result = fake.get_execution_timeline(&ref_, TimelineOpts {}).await;
        assert!(matches!(result, Err(JmError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_execution_timeline_panic() {
        let fake = FakeJourneyManagerPort::new();
        fake.push_get_execution_timeline(Outcome::Panic);
        let ref_ = exec_ref();
        let result = tokio::task::spawn(async move {
            fake.get_execution_timeline(&ref_, TimelineOpts {}).await
        })
        .await;
        assert!(result.is_err());
    }
}
