pub mod metrics;
pub mod policies;
pub mod post_call_http;
pub mod registry_grpc;
pub use post_call_http::PostCallHttpAdapter;
pub use registry_grpc::RegistryGrpcAdapter;

#[cfg(feature = "registry-stub")]
pub mod registry_stub;
#[cfg(feature = "registry-stub")]
pub use registry_stub::RegistryStubAdapter;
