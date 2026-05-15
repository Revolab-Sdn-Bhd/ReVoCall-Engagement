use crate::{error::*, types::*};
use async_trait::async_trait;

#[async_trait]
pub trait RegistryPort: Send + Sync {
    async fn resolve_snapshot(
        &self,
        req: ResolveSnapshotReq,
    ) -> Result<ResolvedSnapshot, RegistryError>;
    async fn get_voice_profile(&self, id: &VoiceProfileId) -> Result<VoiceProfile, RegistryError>;
}

#[async_trait]
pub trait JourneyManagerPort: Send + Sync {
    async fn create_execution(&self, req: CreateExecutionReq) -> Result<ExecutionRef, JmError>;
    async fn cancel_execution(
        &self,
        ref_: &ExecutionRef,
        reason: CancelReason,
    ) -> Result<(), JmError>;
    async fn get_execution_timeline(
        &self,
        ref_: &ExecutionRef,
        opts: TimelineOpts,
    ) -> Result<Timeline, JmError>;
}

#[async_trait]
pub trait VoiceManagerPort: Send + Sync {
    async fn start_voice_session(
        &self,
        req: StartVoiceSessionReq,
    ) -> Result<VoiceSessionRef, VmError>;
    async fn stop_voice_session(
        &self,
        ref_: &VoiceSessionRef,
        mode: StopMode,
    ) -> Result<(), VmError>;
    async fn issue_test_token(&self, req: IssueTestTokenReq) -> Result<TestToken, VmError>;
    // Telephony CRUD
    async fn create_telephony(&self, req: CreateTelephonyReq) -> Result<Telephony, VmError>;
    async fn list_telephonies(&self, req: ListTelephoniesReq) -> Result<Vec<Telephony>, VmError>;
    async fn get_telephony(&self, id: &TelephonyId) -> Result<Telephony, VmError>;
    async fn update_telephony(&self, req: UpdateTelephonyReq) -> Result<Telephony, VmError>;
    async fn delete_telephony(&self, id: &TelephonyId, usage: &str) -> Result<(), VmError>;
}

#[async_trait]
pub trait PostCallPort: Send + Sync {
    async fn get_transcript(&self, eng: &EngagementId) -> Result<Transcript, PostCallError>;
    async fn get_summary(&self, eng: &EngagementId) -> Result<Summary, PostCallError>;
    async fn get_sentiment(&self, eng: &EngagementId) -> Result<Sentiment, PostCallError>;
    async fn get_output_extraction(
        &self,
        eng: &EngagementId,
    ) -> Result<OutputExtraction, PostCallError>;
    async fn list_agent_call_logs(
        &self,
        req: ListAgentCallLogsReq,
    ) -> Result<Page<CallLog>, PostCallError>;
    async fn list_org_call_logs(
        &self,
        req: ListOrgCallLogsReq,
    ) -> Result<Page<CallLog>, PostCallError>;
}

#[async_trait]
pub trait AnalyticsPort: Send + Sync {
    async fn get_agent_analytics(
        &self,
        req: GetAgentAnalyticsReq,
    ) -> Result<Analytics, AnalyticsError>;
    async fn get_agent_metrics(&self, req: GetAgentMetricsReq) -> Result<Metrics, AnalyticsError>;
    async fn get_org_analytics(&self, req: GetOrgAnalyticsReq)
    -> Result<Analytics, AnalyticsError>;
    async fn get_org_metrics(&self, req: GetOrgMetricsReq) -> Result<Metrics, AnalyticsError>;
}
