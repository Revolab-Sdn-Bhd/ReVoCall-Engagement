//! Fake implementation of [`PostCallPort`] for testing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{
    error::PostCallError,
    ports::PostCallPort,
    types::{
        CallLog, EngagementId, ListAgentCallLogsReq, ListOrgCallLogsReq, OutputExtraction, Page,
        Sentiment, Summary, Transcript,
    },
};
use super::Outcome;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct FakePostCallInner {
    get_transcript: VecDeque<Outcome<Transcript>>,
    get_summary: VecDeque<Outcome<Summary>>,
    get_sentiment: VecDeque<Outcome<Sentiment>>,
    get_output_extraction: VecDeque<Outcome<OutputExtraction>>,
    list_agent_call_logs: VecDeque<Outcome<Page<CallLog>>>,
    list_org_call_logs: VecDeque<Outcome<Page<CallLog>>>,
}

// ---------------------------------------------------------------------------
// Public fake
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FakePostCallPort {
    inner: Arc<Mutex<FakePostCallInner>>,
}

impl FakePostCallPort {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakePostCallInner {
                get_transcript: VecDeque::new(),
                get_summary: VecDeque::new(),
                get_sentiment: VecDeque::new(),
                get_output_extraction: VecDeque::new(),
                list_agent_call_logs: VecDeque::new(),
                list_org_call_logs: VecDeque::new(),
            })),
        }
    }

    pub fn push_get_transcript(&self, outcome: Outcome<Transcript>) {
        self.inner.lock().unwrap().get_transcript.push_back(outcome);
    }

    pub fn push_get_summary(&self, outcome: Outcome<Summary>) {
        self.inner.lock().unwrap().get_summary.push_back(outcome);
    }

    pub fn push_get_sentiment(&self, outcome: Outcome<Sentiment>) {
        self.inner.lock().unwrap().get_sentiment.push_back(outcome);
    }

    pub fn push_get_output_extraction(&self, outcome: Outcome<OutputExtraction>) {
        self.inner.lock().unwrap().get_output_extraction.push_back(outcome);
    }

    pub fn push_list_agent_call_logs(&self, outcome: Outcome<Page<CallLog>>) {
        self.inner.lock().unwrap().list_agent_call_logs.push_back(outcome);
    }

    pub fn push_list_org_call_logs(&self, outcome: Outcome<Page<CallLog>>) {
        self.inner.lock().unwrap().list_org_call_logs.push_back(outcome);
    }
}

#[async_trait]
impl PostCallPort for FakePostCallPort {
    async fn get_transcript(
        &self,
        _eng: &EngagementId,
    ) -> Result<Transcript, PostCallError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_transcript
            .pop_front()
            .expect("FakePostCallPort::get_transcript has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(PostCallError::Transient(msg)),
            Outcome::Permanent(msg) => Err(PostCallError::Permanent(msg)),
            Outcome::Unavailable => Err(PostCallError::Unavailable),
            Outcome::Panic => panic!("FakePostCallPort::get_transcript panic injected"),
        }
    }

    async fn get_summary(
        &self,
        _eng: &EngagementId,
    ) -> Result<Summary, PostCallError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_summary
            .pop_front()
            .expect("FakePostCallPort::get_summary has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(PostCallError::Transient(msg)),
            Outcome::Permanent(msg) => Err(PostCallError::Permanent(msg)),
            Outcome::Unavailable => Err(PostCallError::Unavailable),
            Outcome::Panic => panic!("FakePostCallPort::get_summary panic injected"),
        }
    }

    async fn get_sentiment(
        &self,
        _eng: &EngagementId,
    ) -> Result<Sentiment, PostCallError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_sentiment
            .pop_front()
            .expect("FakePostCallPort::get_sentiment has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(PostCallError::Transient(msg)),
            Outcome::Permanent(msg) => Err(PostCallError::Permanent(msg)),
            Outcome::Unavailable => Err(PostCallError::Unavailable),
            Outcome::Panic => panic!("FakePostCallPort::get_sentiment panic injected"),
        }
    }

    async fn get_output_extraction(
        &self,
        _eng: &EngagementId,
    ) -> Result<OutputExtraction, PostCallError> {
        match self
            .inner
            .lock()
            .unwrap()
            .get_output_extraction
            .pop_front()
            .expect("FakePostCallPort::get_output_extraction has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(PostCallError::Transient(msg)),
            Outcome::Permanent(msg) => Err(PostCallError::Permanent(msg)),
            Outcome::Unavailable => Err(PostCallError::Unavailable),
            Outcome::Panic => panic!("FakePostCallPort::get_output_extraction panic injected"),
        }
    }

    async fn list_agent_call_logs(
        &self,
        _req: ListAgentCallLogsReq,
    ) -> Result<Page<CallLog>, PostCallError> {
        match self
            .inner
            .lock()
            .unwrap()
            .list_agent_call_logs
            .pop_front()
            .expect("FakePostCallPort::list_agent_call_logs has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(PostCallError::Transient(msg)),
            Outcome::Permanent(msg) => Err(PostCallError::Permanent(msg)),
            Outcome::Unavailable => Err(PostCallError::Unavailable),
            Outcome::Panic => panic!("FakePostCallPort::list_agent_call_logs panic injected"),
        }
    }

    async fn list_org_call_logs(
        &self,
        _req: ListOrgCallLogsReq,
    ) -> Result<Page<CallLog>, PostCallError> {
        match self
            .inner
            .lock()
            .unwrap()
            .list_org_call_logs
            .pop_front()
            .expect("FakePostCallPort::list_org_call_logs has no queued response")
        {
            Outcome::Success(v) => Ok(v),
            Outcome::Transient(msg) => Err(PostCallError::Transient(msg)),
            Outcome::Permanent(msg) => Err(PostCallError::Permanent(msg)),
            Outcome::Unavailable => Err(PostCallError::Unavailable),
            Outcome::Panic => panic!("FakePostCallPort::list_org_call_logs panic injected"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn eng_id() -> EngagementId {
        EngagementId::new()
    }

    fn empty_page() -> Page<CallLog> {
        Page { items: vec![], next_page_token: None }
    }

    // --- get_transcript ---

    #[tokio::test]
    async fn get_transcript_success() {
        let fake = FakePostCallPort::new();
        fake.push_get_transcript(Outcome::Success(Transcript { text: "hello".into() }));
        let result = fake.get_transcript(&eng_id()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_transcript_transient() {
        let fake = FakePostCallPort::new();
        fake.push_get_transcript(Outcome::Transient("timeout".into()));
        let result = fake.get_transcript(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Transient(_))));
    }

    #[tokio::test]
    async fn get_transcript_permanent() {
        let fake = FakePostCallPort::new();
        fake.push_get_transcript(Outcome::Permanent("not found".into()));
        let result = fake.get_transcript(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_transcript_unavailable() {
        let fake = FakePostCallPort::new();
        fake.push_get_transcript(Outcome::Unavailable);
        let result = fake.get_transcript(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Unavailable)));
    }

    #[tokio::test]
    async fn get_transcript_panic() {
        let fake = FakePostCallPort::new();
        fake.push_get_transcript(Outcome::Panic);
        let id = eng_id();
        let result = tokio::task::spawn(async move {
            fake.get_transcript(&id).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- get_summary ---

    #[tokio::test]
    async fn get_summary_success() {
        let fake = FakePostCallPort::new();
        fake.push_get_summary(Outcome::Success(Summary { text: "summary".into() }));
        let result = fake.get_summary(&eng_id()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_summary_transient() {
        let fake = FakePostCallPort::new();
        fake.push_get_summary(Outcome::Transient("timeout".into()));
        let result = fake.get_summary(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Transient(_))));
    }

    #[tokio::test]
    async fn get_summary_permanent() {
        let fake = FakePostCallPort::new();
        fake.push_get_summary(Outcome::Permanent("not found".into()));
        let result = fake.get_summary(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_summary_panic() {
        let fake = FakePostCallPort::new();
        fake.push_get_summary(Outcome::Panic);
        let id = eng_id();
        let result = tokio::task::spawn(async move {
            fake.get_summary(&id).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- get_sentiment ---

    #[tokio::test]
    async fn get_sentiment_success() {
        let fake = FakePostCallPort::new();
        fake.push_get_sentiment(Outcome::Success(Sentiment { score: 0.9 }));
        let result = fake.get_sentiment(&eng_id()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_sentiment_transient() {
        let fake = FakePostCallPort::new();
        fake.push_get_sentiment(Outcome::Transient("timeout".into()));
        let result = fake.get_sentiment(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Transient(_))));
    }

    #[tokio::test]
    async fn get_sentiment_permanent() {
        let fake = FakePostCallPort::new();
        fake.push_get_sentiment(Outcome::Permanent("not found".into()));
        let result = fake.get_sentiment(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_sentiment_panic() {
        let fake = FakePostCallPort::new();
        fake.push_get_sentiment(Outcome::Panic);
        let id = eng_id();
        let result = tokio::task::spawn(async move {
            fake.get_sentiment(&id).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- get_output_extraction ---

    #[tokio::test]
    async fn get_output_extraction_success() {
        let fake = FakePostCallPort::new();
        fake.push_get_output_extraction(Outcome::Success(OutputExtraction {}));
        let result = fake.get_output_extraction(&eng_id()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_output_extraction_transient() {
        let fake = FakePostCallPort::new();
        fake.push_get_output_extraction(Outcome::Transient("timeout".into()));
        let result = fake.get_output_extraction(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Transient(_))));
    }

    #[tokio::test]
    async fn get_output_extraction_permanent() {
        let fake = FakePostCallPort::new();
        fake.push_get_output_extraction(Outcome::Permanent("not found".into()));
        let result = fake.get_output_extraction(&eng_id()).await;
        assert!(matches!(result, Err(PostCallError::Permanent(_))));
    }

    #[tokio::test]
    async fn get_output_extraction_panic() {
        let fake = FakePostCallPort::new();
        fake.push_get_output_extraction(Outcome::Panic);
        let id = eng_id();
        let result = tokio::task::spawn(async move {
            fake.get_output_extraction(&id).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- list_agent_call_logs ---

    #[tokio::test]
    async fn list_agent_call_logs_success() {
        let fake = FakePostCallPort::new();
        fake.push_list_agent_call_logs(Outcome::Success(empty_page()));
        let result = fake.list_agent_call_logs(ListAgentCallLogsReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn list_agent_call_logs_transient() {
        let fake = FakePostCallPort::new();
        fake.push_list_agent_call_logs(Outcome::Transient("timeout".into()));
        let result = fake.list_agent_call_logs(ListAgentCallLogsReq {}).await;
        assert!(matches!(result, Err(PostCallError::Transient(_))));
    }

    #[tokio::test]
    async fn list_agent_call_logs_permanent() {
        let fake = FakePostCallPort::new();
        fake.push_list_agent_call_logs(Outcome::Permanent("forbidden".into()));
        let result = fake.list_agent_call_logs(ListAgentCallLogsReq {}).await;
        assert!(matches!(result, Err(PostCallError::Permanent(_))));
    }

    #[tokio::test]
    async fn list_agent_call_logs_panic() {
        let fake = FakePostCallPort::new();
        fake.push_list_agent_call_logs(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.list_agent_call_logs(ListAgentCallLogsReq {}).await
        })
        .await;
        assert!(result.is_err());
    }

    // --- list_org_call_logs ---

    #[tokio::test]
    async fn list_org_call_logs_success() {
        let fake = FakePostCallPort::new();
        fake.push_list_org_call_logs(Outcome::Success(empty_page()));
        let result = fake.list_org_call_logs(ListOrgCallLogsReq {}).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn list_org_call_logs_transient() {
        let fake = FakePostCallPort::new();
        fake.push_list_org_call_logs(Outcome::Transient("timeout".into()));
        let result = fake.list_org_call_logs(ListOrgCallLogsReq {}).await;
        assert!(matches!(result, Err(PostCallError::Transient(_))));
    }

    #[tokio::test]
    async fn list_org_call_logs_permanent() {
        let fake = FakePostCallPort::new();
        fake.push_list_org_call_logs(Outcome::Permanent("forbidden".into()));
        let result = fake.list_org_call_logs(ListOrgCallLogsReq {}).await;
        assert!(matches!(result, Err(PostCallError::Permanent(_))));
    }

    #[tokio::test]
    async fn list_org_call_logs_panic() {
        let fake = FakePostCallPort::new();
        fake.push_list_org_call_logs(Outcome::Panic);
        let result = tokio::task::spawn(async move {
            fake.list_org_call_logs(ListOrgCallLogsReq {}).await
        })
        .await;
        assert!(result.is_err());
    }
}
