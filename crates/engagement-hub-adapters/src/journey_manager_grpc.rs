use std::sync::Arc;

use async_trait::async_trait;
use engagement_hub_ports::{
    error::JmError,
    ports::JourneyManagerPort,
    types::{
        CancelReason, CreateExecutionReq, ExecutionRef, Timeline, TimelineEvent, TimelineOpts,
    },
};
use tonic::{Code, transport::Channel};
use uuid::Uuid;

use crate::{
    metrics::AdapterMetrics,
    policies::{CLEANUP_RETRY, DEFAULT_RETRY, WRITE_RETRY, with_retry},
};

mod proto {
    tonic::include_proto!("revocall.journey.v1");
}
use proto::journey_manager_client::JourneyManagerClient;

fn map_status(s: tonic::Status) -> JmError {
    match s.code() {
        Code::NotFound
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::Unimplemented
        | Code::OutOfRange
        | Code::Cancelled => JmError::Permanent(format!("{:?}: {}", s.code(), s.message())),
        Code::Unavailable => JmError::Unavailable,
        _ => JmError::Transient(format!("{:?}: {}", s.code(), s.message())),
    }
}

fn cancel_reason_to_proto(r: CancelReason) -> proto::CancelReason {
    match r {
        CancelReason::CompensateFailedBind => proto::CancelReason::CompensateFailedBind,
        CancelReason::UserRequested => proto::CancelReason::UserRequested,
        CancelReason::OrchestratorTimeout => proto::CancelReason::OrchestratorTimeout,
        CancelReason::AdminCancelled => proto::CancelReason::AdminCancelled,
    }
}

pub struct JourneyManagerGrpcAdapter {
    client: JourneyManagerClient<Channel>,
    metrics: Arc<AdapterMetrics>,
}

impl JourneyManagerGrpcAdapter {
    pub fn new(channel: Channel, metrics: Arc<AdapterMetrics>) -> Self {
        Self {
            client: JourneyManagerClient::new(channel),
            metrics,
        }
    }
}

// Trait impl methods filled in in Tasks 10–12.

#[cfg(test)]
mod tests {
    // Shared test harness — populated as methods are implemented.
}
