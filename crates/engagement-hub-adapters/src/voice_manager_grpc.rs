use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::VmError,
    ports::VoiceManagerPort,
    types::{
        CreateTelephonyReq, IssueTestTokenReq, ListTelephoniesReq, StartVoiceSessionReq, StopMode,
        Telephony, TelephonyId, TestToken, UpdateTelephonyReq, VoiceSessionRef,
    },
};
use tonic::{Code, transport::Channel};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{
        CLEANUP_RETRY, DEFAULT_RETRY, RetryConfig, WRITE_RETRY, with_retry,
    },
};

mod proto {
    tonic::include_proto!("revocall.voice.v1");
}
use proto::voice_manager_client::VoiceManagerClient;

fn map_status(s: tonic::Status) -> VmError {
    match s.code() {
        Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::Cancelled => VmError::Permanent(format!("{:?}: {}", s.code(), s.message())),
        Code::Unavailable => VmError::Unavailable,
        _ => VmError::Transient(format!("{:?}: {}", s.code(), s.message())),
    }
}

fn stop_mode_to_proto(m: &StopMode) -> proto::StopMode {
    match m {
        StopMode::Abort => proto::StopMode::Abort,
        StopMode::Graceful => proto::StopMode::Graceful,
    }
}

/// 1 attempt — used for stop_voice_session(mode=Graceful), which is NOT idempotent.
const GRACEFUL_STOP_RETRY: RetryConfig = RetryConfig {
    max_attempts: 1,
    initial_backoff: std::time::Duration::from_millis(50),
    max_backoff: std::time::Duration::from_secs(2),
};

fn telephony_from_proto(t: proto::TelephonyProto) -> Result<Telephony, VmError> {
    let id = t.id.parse::<Uuid>()
        .map(TelephonyId::from)
        .map_err(|e| VmError::Permanent(format!("bad telephony id: {e}")))?;
    Ok(Telephony {
        id,
        org_id: t.org_id,
        phone_number: t.phone_number,
    })
}

pub struct VoiceManagerGrpcAdapter {
    client: VoiceManagerClient<Channel>,
    metrics: Arc<AdapterMetrics>,
}

impl VoiceManagerGrpcAdapter {
    pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client: VoiceManagerClient::new(channel),
            metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    // Populated by Task 15.
}
