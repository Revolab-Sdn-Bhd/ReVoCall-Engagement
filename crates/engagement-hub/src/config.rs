use std::net::SocketAddr;

use clap::{Parser, ValueEnum};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    Json,
    Pretty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Env {
    Dev,
    Staging,
    Production,
}

impl Env {
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Env::Dev => "dev",
            Env::Staging => "staging",
            Env::Production => "production",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum RegistryAdapter {
    Stub,
    Grpc,
}

impl RegistryAdapter {
    pub fn as_metric_label(self) -> &'static str {
        match self {
            RegistryAdapter::Stub => "stub",
            RegistryAdapter::Grpc => "grpc",
        }
    }
}

#[derive(Clone, Debug, Parser)]
#[command(name = "engagement-hub", version)]
pub struct Config {
    #[arg(long, env = "EH_ENV", value_enum)]
    pub env: Env,

    #[arg(long, env = "EH_REGISTRY_ADAPTER", value_enum)]
    pub registry_adapter: RegistryAdapter,

    #[arg(long, env = "EH_TRACK_0_IDLE_MODE", default_value_t = false)]
    pub track_0_idle_mode: bool,

    #[arg(long, env = "EH_DATABASE_URL")]
    pub database_url: String,

    #[arg(long, env = "EH_EXTERNAL_GRPC_ADDR", default_value = "0.0.0.0:8443")]
    pub external_grpc_addr: SocketAddr,

    #[arg(long, env = "EH_INTERNAL_GRPC_ADDR", default_value = "0.0.0.0:8444")]
    pub internal_grpc_addr: SocketAddr,

    #[arg(long, env = "EH_HTTP_ADDR", default_value = "0.0.0.0:9090")]
    pub http_addr: SocketAddr,

    #[arg(long, env = "EH_DB_POOL_MIN", default_value_t = 10)]
    pub db_pool_min: u32,

    #[arg(long, env = "EH_DB_POOL_MAX", default_value_t = 25)]
    pub db_pool_max: u32,

    #[arg(long, env = "EH_DB_IDLE_TIMEOUT_SECS", default_value_t = 300)]
    pub db_idle_timeout_secs: u64,

    #[arg(long, env = "EH_DB_STATEMENT_TIMEOUT_MS", default_value_t = 5000)]
    pub db_statement_timeout_ms: u64,

    #[arg(long, env = "EH_DB_SLOW_QUERY_MS", default_value_t = 500)]
    pub db_slow_query_ms: u64,

    #[arg(long, env = "EH_LOG_FORMAT", value_enum, default_value_t = LogFormat::Json)]
    pub log_format: LogFormat,
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("EH_REGISTRY_ADAPTER=stub is forbidden in production unless EH_TRACK_0_IDLE_MODE=true")]
    ProdStubWithoutIdle,
    #[error("EH_DB_STATEMENT_TIMEOUT_MS=0 disables the statement timeout entirely; set to >= 1")]
    StatementTimeoutDisabled,
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.env == Env::Production
            && self.registry_adapter == RegistryAdapter::Stub
            && !self.track_0_idle_mode
        {
            return Err(ConfigError::ProdStubWithoutIdle);
        }
        if self.db_statement_timeout_ms == 0 {
            return Err(ConfigError::StatementTimeoutDisabled);
        }
        Ok(())
    }

    pub fn bind_external_port(&self) -> bool {
        !self.track_0_idle_mode
    }
}
