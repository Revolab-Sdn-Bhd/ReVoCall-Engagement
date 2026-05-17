// Verifies adapter structs can be constructed without panicking.

use engagement_hub_adapters::{AnalyticsHttpAdapter, PostCallHttpAdapter, metrics::AdapterMetrics};
use reqwest::Client;

#[test]
fn post_call_http_adapter_constructs() {
    let _ = PostCallHttpAdapter::new(
        Client::new(),
        "http://localhost:9999".into(),
        AdapterMetrics::for_test(),
    );
}

#[test]
fn analytics_http_adapter_constructs() {
    let _ = AnalyticsHttpAdapter::new(
        Client::new(),
        "http://localhost:9999".into(),
        AdapterMetrics::for_test(),
    );
}

#[cfg(feature = "registry-stub")]
#[test]
fn registry_stub_adapter_constructs() {
    use engagement_hub_adapters::RegistryStubAdapter;
    let _ = RegistryStubAdapter::with_default_fixtures();
}
