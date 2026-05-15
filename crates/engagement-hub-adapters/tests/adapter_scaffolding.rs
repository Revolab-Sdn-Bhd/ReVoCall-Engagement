// Integration test scaffolding for T1-03+
// These tests verify adapter struct construction and will be expanded
// when real implementations land in T1-03.
//
// Note: to run tests with fake port implementations, use:
//   cargo test -p engagement-hub-adapters --features engagement-hub-ports/fake

use engagement_hub_adapters::{
    AnalyticsHttpAdapter, JourneyManagerGrpcAdapter, PostCallHttpAdapter, RegistryGrpcAdapter,
    VoiceManagerHttpAdapter,
};

// Compile-time check: all adapter types are importable
// (actual behavior tests added in T1-03)

#[test]
#[should_panic]
fn registry_grpc_adapter_new_panics_until_t1_03() {
    let _ = RegistryGrpcAdapter::new();
}

#[test]
#[should_panic]
fn journey_manager_grpc_adapter_new_panics_until_t1_03() {
    let _ = JourneyManagerGrpcAdapter::new();
}

#[test]
#[should_panic]
fn voice_manager_http_adapter_new_panics_until_t1_03() {
    let _ = VoiceManagerHttpAdapter::new();
}

#[test]
#[should_panic]
fn post_call_http_adapter_new_panics_until_t1_03() {
    let _ = PostCallHttpAdapter::new();
}

#[test]
#[should_panic]
fn analytics_http_adapter_new_panics_until_t1_03() {
    let _ = AnalyticsHttpAdapter::new();
}
