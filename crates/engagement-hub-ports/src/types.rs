//! Rust-native domain types used in the five port trait signatures.
//!
//! These are NOT proto-generated types. Adapter crates (T1-03+) will reconcile
//! to proto types internally. Fields are intentionally minimal; they will be
//! filled out in T1-03+.

#![allow(dead_code)]

use uuid::Uuid;

// ---------------------------------------------------------------------------
// ID newtypes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EngagementId(pub Uuid);

impl EngagementId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for EngagementId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VoiceProfileId(pub Uuid);

impl VoiceProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for VoiceProfileId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TelephonyId(pub Uuid);

impl TelephonyId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for TelephonyId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

// ---------------------------------------------------------------------------
// RegistryPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResolveSnapshotReq {}

#[derive(Debug, Clone)]
pub struct ResolvedSnapshot {}

#[derive(Debug, Clone)]
pub struct VoiceProfile {}

// ---------------------------------------------------------------------------
// JourneyManagerPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CreateExecutionReq {}

#[derive(Debug, Clone)]
pub struct ExecutionRef {
    pub id: Uuid,
}

#[derive(Debug, Clone)]
pub enum CancelReason {
    CompensateFailedBind,
    UserRequested,
    OrchestratorTimeout,
}

#[derive(Debug, Clone)]
pub struct TimelineOpts {}

#[derive(Debug, Clone)]
pub struct Timeline {}

// ---------------------------------------------------------------------------
// VoiceManagerPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StartVoiceSessionReq {}

#[derive(Debug, Clone)]
pub struct VoiceSessionRef {
    pub id: Uuid,
}

#[derive(Debug, Clone)]
pub enum StopMode {
    Abort,
    Graceful,
}

#[derive(Debug, Clone)]
pub struct IssueTestTokenReq {}

#[derive(Debug, Clone)]
pub struct TestToken {
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct CreateTelephonyReq {}

#[derive(Debug, Clone)]
pub struct Telephony {
    pub id: TelephonyId,
}

#[derive(Debug, Clone)]
pub struct ListTelephoniesReq {}

#[derive(Debug, Clone)]
pub struct UpdateTelephonyReq {}

// ---------------------------------------------------------------------------
// PostCallPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Sentiment {
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct OutputExtraction {}

#[derive(Debug, Clone)]
pub struct ListAgentCallLogsReq {}

#[derive(Debug, Clone)]
pub struct ListOrgCallLogsReq {}

#[derive(Debug, Clone)]
pub struct CallLog {}

#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: Option<String>,
}

// ---------------------------------------------------------------------------
// AnalyticsPort types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GetAgentAnalyticsReq {}

#[derive(Debug, Clone)]
pub struct GetAgentMetricsReq {}

#[derive(Debug, Clone)]
pub struct GetOrgAnalyticsReq {}

#[derive(Debug, Clone)]
pub struct GetOrgMetricsReq {}

#[derive(Debug, Clone)]
pub struct Analytics {}

#[derive(Debug, Clone)]
pub struct Metrics {}
