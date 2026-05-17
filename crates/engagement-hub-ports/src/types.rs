//! Rust-native domain types used in the five port trait signatures.
//!
//! These are NOT proto-generated types. Adapter crates (T1-03+) will reconcile
//! to proto types internally.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// ID newtypes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EngagementId(Uuid);

impl EngagementId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn into_uuid(self) -> Uuid {
        self.0
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for EngagementId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for EngagementId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for EngagementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VoiceProfileId(Uuid);

impl VoiceProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn into_uuid(self) -> Uuid {
        self.0
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for VoiceProfileId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for VoiceProfileId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for VoiceProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TelephonyId(Uuid);

impl TelephonyId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn into_uuid(self) -> Uuid {
        self.0
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for TelephonyId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for TelephonyId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for TelephonyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// RegistryPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveSnapshotReq {
    pub org_id: String,
    pub journey_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSnapshot {
    pub snapshot_id: String,
    pub journey_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceProfile {
    pub id: VoiceProfileId,
    pub name: String,
}

// ---------------------------------------------------------------------------
// JourneyManagerPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExecutionReq {
    pub journey_version: String,
    pub org_id: String,
    pub engagement_id: EngagementId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRef {
    id: Uuid,
}

impl ExecutionRef {
    pub fn new(id: Uuid) -> Self {
        Self { id }
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.id
    }

    pub fn into_uuid(self) -> Uuid {
        self.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CancelReason {
    CompensateFailedBind,
    UserRequested,
    OrchestratorTimeout,
    AdminCancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineOpts {
    pub after_sequence: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub sequence: u64,
    pub kind: String,
}

// ---------------------------------------------------------------------------
// VoiceManagerPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartVoiceSessionReq {
    pub engagement_id: EngagementId,
    pub org_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSessionRef {
    id: Uuid,
}

impl VoiceSessionRef {
    pub fn new(id: Uuid) -> Self {
        Self { id }
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.id
    }

    pub fn into_uuid(self) -> Uuid {
        self.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StopMode {
    /// Best-effort immediate teardown. Idempotent — safe to call even if session already gone.
    Abort,
    /// Coordinated teardown allowing in-flight audio to drain. Not idempotent.
    Graceful,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueTestTokenReq {
    pub org_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestToken {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTelephonyReq {
    pub org_id: String,
    pub phone_number: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Telephony {
    pub id: TelephonyId,
    pub org_id: String,
    pub phone_number: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTelephoniesReq {
    pub org_id: String,
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTelephonyReq {
    pub id: TelephonyId,
    pub phone_number: String,
}

// ---------------------------------------------------------------------------
// PostCallPort types
// Shapes match admin-backend/cmd/server/admin/calllog/types.go
// ---------------------------------------------------------------------------

/// Structured transcription. Adapter concatenates messages into `text` for
/// callers that need a flat string; the full list is in `messages`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub messages: Vec<TranscriptMessage>,
    pub total_size: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptMessage {
    pub message: String,
    pub role: String,
    pub audio_url: Option<String>,
    pub emotion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub summary: String,
    pub resolution: Option<String>,
    pub resolution_explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sentiment {
    pub label: String, // "positive" | "negative" | "neutral"
    pub justification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputExtraction {
    pub fields: Vec<OutputField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputField {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAgentCallLogsReq {
    pub agent_id: String,
    pub skip: Option<u32>,
    pub limit: Option<u32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub identity: Option<String>,
    pub id: Option<String>,
    pub batch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListOrgCallLogsReq {
    pub org_id: String,
    pub skip: Option<u32>,
    pub limit: Option<u32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub contact_number: Option<String>,
    pub call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallLog {
    pub id: String,
    pub room_name: Option<String>,
    pub batch_id: Option<String>,
    pub duration: Option<i32>,
    pub identity: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total_size: Option<u32>,
    pub next_page_token: Option<String>,
}

// ---------------------------------------------------------------------------
// AnalyticsPort types
// Shapes match admin-backend/cmd/server/admin/analytic/types.go
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAgentAnalyticsReq {
    pub agent_id: String,
    pub metric: Option<String>,
    pub granularity: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAgentMetricsReq {
    pub agent_id: String,
    pub metric: Option<String>,
    pub granularity: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOrgAnalyticsReq {
    pub org_id: String,
    pub metric: Option<String>,
    pub granularity: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOrgMetricsReq {
    pub org_id: String,
    pub metric: Option<String>,
    pub granularity: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analytics {
    pub average_conversation_duration: f64,
    pub containment_rate: f64,
    pub customer_satisfaction_rate: f64,
    pub dropoff_rate: f64,
    pub escalation_rate: f64,
    pub total_inquiries: u32,
    pub category_counts: HashMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub categories: Vec<String>,
    pub series: Vec<f64>,
}
