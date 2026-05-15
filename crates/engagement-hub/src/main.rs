use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{EnvFilter, prelude::*};

use engagement_hub::{
    config::{Config, ConfigError, LogFormat},
    db,
    metrics::Metrics,
    server::{grpc, http},
    shutdown::{Shutdown, wait_for_signal},
};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::parse();

    if let Err(e) = cfg.validate() {
        eprintln!("invalid config: {e}");
        // ConfigError values map 1:1 to exit code 78 today; expand later when
        // validation grows.
        let _: ConfigError = e;
        std::process::exit(78); // EX_CONFIG
    }

    init_tracing(cfg.log_format);

    let pool = db::build_pool(&cfg).await?;
    db::run_migrations(&pool).await.unwrap_or_else(|err| {
        tracing::error!(?err, "migration run failed");
        std::process::exit(70); // EX_SOFTWARE
    });

    let metrics = Arc::new(Metrics::new(cfg.registry_adapter, cfg.env, cfg.track_0_idle_mode)?);
    let shutdown = Shutdown::default();

    let mut grpc_servers = grpc::spawn(
        cfg.external_grpc_addr,
        cfg.internal_grpc_addr,
        cfg.bind_external_port(),
        shutdown.shutdown_rx.clone(),
    )
    .await?;

    let http_addr = cfg.http_addr;
    let http_metrics = metrics.clone();
    let http_drain_rx = shutdown.drain_rx.clone();
    let http_shutdown_rx = shutdown.shutdown_rx.clone();
    let http_handle = tokio::spawn(async move {
        http::serve(http_addr, http_metrics, http_drain_rx, http_shutdown_rx).await
    });

    tracing::info!(
        env = ?cfg.env,
        adapter = ?cfg.registry_adapter,
        idle = cfg.track_0_idle_mode,
        external = %cfg.external_grpc_addr,
        internal = %cfg.internal_grpc_addr,
        http = %cfg.http_addr,
        "engagement-hub started",
    );

    wait_for_signal().await;

    let _ = shutdown.drain_tx.send(true);
    grpc_servers.signal_draining().await;
    tracing::info!("draining (readyz now reports 503)");
    let _ = shutdown.shutdown_tx.send(true);

    let _ = http_handle.await;
    if let Some(h) = grpc_servers.external_handle {
        let _ = h.await;
    }
    let _ = grpc_servers.internal_handle.await;

    pool.close().await;
    tracing::info!("exited cleanly");
    Ok(())
}

fn init_tracing(format: LogFormat) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,engagement_hub=debug,sqlx::query=warn"));

    let registry = tracing_subscriber::registry().with(env_filter);
    match format {
        LogFormat::Pretty => {
            registry
                .with(tracing_subscriber::fmt::layer().pretty())
                .init();
        }
        LogFormat::Json => {
            registry
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        }
    }
}
