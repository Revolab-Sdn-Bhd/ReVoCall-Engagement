//! Adapter stubs for engagement-hub downstream integrations.
//!
//! These are structural placeholders only. Real implementations land in T1-03+.

// Imports are forward-declared for T1-03; suppress unused warnings on stubs.
#[allow(unused_imports)]
use engagement_hub_ports::{
    types::*,
    error::*,
    ports::*,
};
#[allow(unused_imports)]
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// gRPC adapter for the Registry service.
pub struct RegistryGrpcAdapter {
    // TODO T1-03: add tonic Channel field
}

impl RegistryGrpcAdapter {
    pub fn new() -> Self {
        todo!("not yet implemented — see T1-03")
    }
}

// ---------------------------------------------------------------------------
// Journey Manager
// ---------------------------------------------------------------------------

/// gRPC adapter for the Journey Manager service.
pub struct JourneyManagerGrpcAdapter {
    // TODO T1-03: add tonic Channel field
}

impl JourneyManagerGrpcAdapter {
    pub fn new() -> Self {
        todo!("not yet implemented — see T1-03")
    }
}

// ---------------------------------------------------------------------------
// Voice Manager (HTTP today, gRPC future)
// ---------------------------------------------------------------------------

/// HTTP adapter for the Voice Manager service.
pub struct VoiceManagerHttpAdapter {
    // TODO T1-03: add reqwest::Client + base URL fields
}

impl VoiceManagerHttpAdapter {
    pub fn new() -> Self {
        todo!("not yet implemented — see T1-03")
    }
}

// ---------------------------------------------------------------------------
// Post-Call (HTTP today)
// ---------------------------------------------------------------------------

/// HTTP adapter for the Post-Call service.
pub struct PostCallHttpAdapter {
    // TODO T1-03: add reqwest::Client + base URL fields
}

impl PostCallHttpAdapter {
    pub fn new() -> Self {
        todo!("not yet implemented — see T1-03")
    }
}

// ---------------------------------------------------------------------------
// Analytics (HTTP today)
// ---------------------------------------------------------------------------

/// HTTP adapter for the Analytics service.
pub struct AnalyticsHttpAdapter {
    // TODO T1-03: add reqwest::Client + base URL fields
}

impl AnalyticsHttpAdapter {
    pub fn new() -> Self {
        todo!("not yet implemented — see T1-03")
    }
}
