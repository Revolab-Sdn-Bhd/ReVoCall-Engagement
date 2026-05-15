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

    #[arg(
        long,
        env = "EH_DB_ACQUIRE_TIMEOUT_SECS",
        default_value_t = 3,
        help = "Max seconds to wait for a connection from the pool before error"
    )]
    pub db_acquire_timeout_secs: u64,

    #[arg(
        long,
        env = "EH_DB_MAX_LIFETIME_SECS",
        default_value_t = 1800,
        help = "Max lifetime (seconds) of a pooled connection before recycling (default 30 min)"
    )]
    pub db_max_lifetime_secs: u64,

    #[arg(long, env = "EH_LOG_FORMAT", value_enum, default_value_t = LogFormat::Json)]
    pub log_format: LogFormat,

    // OTEL toggles — no EH_ prefix, platform-wide vars per PRD §10
    #[arg(long, env = "OTEL_EXPORT_GRAFANA", default_value_t = true)]
    pub otel_export_grafana: bool,

    #[arg(long, env = "OTEL_EXPORT_LANGFUSE", default_value_t = false)]
    pub otel_export_langfuse: bool,

    // Clap default is false; apply_otel_local_default() sets true when env=dev and var unset
    #[arg(long, env = "OTEL_EXPORT_LOCAL", default_value_t = false)]
    pub otel_export_local: bool,

    #[arg(
        long,
        env = "OTEL_GRAFANA_ENDPOINT",
        default_value = "http://localhost:4317"
    )]
    pub otel_grafana_endpoint: String,

    #[arg(
        long,
        env = "OTEL_LANGFUSE_ENDPOINT",
        default_value = "https://cloud.langfuse.com/api/public/otel"
    )]
    pub otel_langfuse_endpoint: String,

    #[arg(long, env = "LANGFUSE_PUBLIC_KEY")]
    pub langfuse_public_key: Option<String>,

    #[arg(long, env = "LANGFUSE_SECRET_KEY")]
    pub langfuse_secret_key: Option<String>,
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("EH_REGISTRY_ADAPTER=stub is forbidden in production unless EH_TRACK_0_IDLE_MODE=true")]
    ProdStubWithoutIdle,
    #[error("EH_DB_STATEMENT_TIMEOUT_MS=0 disables the statement timeout entirely; set to >= 1")]
    StatementTimeoutDisabled,
    #[error("OTEL_EXPORT_LANGFUSE=true requires both LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY")]
    LangfuseKeysMissing,
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
        if self.otel_export_langfuse {
            match (&self.langfuse_public_key, &self.langfuse_secret_key) {
                (Some(pk), Some(sk)) if !pk.is_empty() && !sk.is_empty() => {}
                _ => {
                    return Err(ConfigError::LangfuseKeysMissing);
                }
            }
        }
        Ok(())
    }

    pub fn bind_external_port(&self) -> bool {
        !self.track_0_idle_mode
    }
}

/// Sets otel_export_local=true when env=Dev and OTEL_EXPORT_LOCAL was not explicitly set.
/// Pass `var_set = std::env::var_os("OTEL_EXPORT_LOCAL").is_some()`.
pub fn apply_otel_local_default(cfg: &mut Config, var_set: bool) {
    if !var_set && cfg.env == Env::Dev {
        cfg.otel_export_local = true;
    }
}

/// Translates legacy OTEL_TYPE value into new toggle fields.
/// Call after tracing is initialized so the caller can emit a deprecation warning.
pub fn apply_otel_type_legacy(cfg: &mut Config, otel_type: Option<&str>) {
    match otel_type {
        Some("collector") => cfg.otel_export_grafana = true,
        Some("langfuse") => cfg.otel_export_langfuse = true,
        Some("both") => {
            cfg.otel_export_grafana = true;
            cfg.otel_export_langfuse = true;
        }
        Some(other) => {
            eprintln!(
                "[otel] unknown OTEL_TYPE value {other:?}; expected: collector, langfuse, both"
            );
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn otel_export_local_defaults_true_when_env_is_dev_and_var_not_set() {
        let mut cfg = Config::try_parse_from([
            "engagement-hub",
            "--env",
            "dev",
            "--registry-adapter",
            "stub",
            "--database-url",
            "postgres://localhost/test",
        ])
        .unwrap();
        apply_otel_local_default(&mut cfg, false);
        assert!(cfg.otel_export_local, "expected local=true for dev env");
    }

    #[test]
    fn otel_export_local_not_overridden_when_explicitly_set() {
        let mut cfg = Config::try_parse_from([
            "engagement-hub",
            "--env",
            "dev",
            "--registry-adapter",
            "stub",
            "--database-url",
            "postgres://localhost/test",
        ])
        .unwrap();
        // var_set=true means OTEL_EXPORT_LOCAL was explicitly set; default (false) must be preserved
        apply_otel_local_default(&mut cfg, true);
        assert!(!cfg.otel_export_local);
    }

    #[test]
    fn otel_type_both_sets_grafana_and_langfuse() {
        let mut cfg = Config::try_parse_from([
            "engagement-hub",
            "--env",
            "dev",
            "--registry-adapter",
            "stub",
            "--database-url",
            "postgres://localhost/test",
        ])
        .unwrap();
        apply_otel_type_legacy(&mut cfg, Some("both"));
        assert!(cfg.otel_export_grafana);
        assert!(cfg.otel_export_langfuse);
    }

    #[test]
    fn otel_type_collector_sets_grafana_only() {
        let mut cfg = Config::try_parse_from([
            "engagement-hub",
            "--env",
            "dev",
            "--registry-adapter",
            "stub",
            "--database-url",
            "postgres://localhost/test",
        ])
        .unwrap();
        apply_otel_type_legacy(&mut cfg, Some("collector"));
        assert!(cfg.otel_export_grafana);
        assert!(!cfg.otel_export_langfuse);
    }
}
