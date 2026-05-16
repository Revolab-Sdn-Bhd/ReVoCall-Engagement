pub mod metrics;
pub mod policies;
pub mod registry_grpc;
pub use registry_grpc::RegistryGrpcAdapter;

#[cfg(feature = "registry-stub")]
pub mod registry_stub;
#[cfg(feature = "registry-stub")]
pub use registry_stub::RegistryStubAdapter;
