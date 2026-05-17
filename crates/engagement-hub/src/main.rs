use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use engagement_hub::{
    config::{
        Config, ConfigError, RegistryAdapter, apply_otel_local_default, apply_otel_type_legacy,
    },
    db,
    metrics::Metrics,
    notify::ListenNotifyManager,
    server::{grpc, http},
    shutdown::{Shutdown, wait_for_signal},
    telemetry,
};

#[tokio::main]
async fn main() -> Result<()> {
    let mut cfg = Config::parse();

    if let Err(e) = cfg.validate() {
        eprintln!("invalid config: {e}");
        let _: ConfigError = e;
        std::process::exit(78);
    }

    apply_otel_local_default(&mut cfg, std::env::var_os("OTEL_EXPORT_LOCAL").is_some());

    // Legacy OTEL_TYPE deprecation (before init so translation actually takes effect)
    let otel_type = std::env::var("OTEL_TYPE").ok();
    if let Some(ref t) = otel_type {
        eprintln!(
            "[otel] OTEL_TYPE={t} is deprecated; use OTEL_EXPORT_GRAFANA / OTEL_EXPORT_LANGFUSE / OTEL_EXPORT_LOCAL"
        );
        apply_otel_type_legacy(&mut cfg, Some(t.as_str()));
        // Re-validate after legacy translation may have enabled exporters that require credentials
        if let Err(e) = cfg.validate() {
            eprintln!("invalid config after OTEL_TYPE translation: {e}");
            let _: ConfigError = e;
            std::process::exit(78);
        }
    }

    let metrics = Arc::new(Metrics::new(
        cfg.registry_adapter,
        cfg.env,
        cfg.track_0_idle_mode,
    )?);

    telemetry::init_telemetry(&cfg, &metrics);

    // After telemetry is up, emit structured warn so it appears in logs
    if let Some(ref t) = otel_type {
        tracing::warn!(
            otel_type = %t,
            "OTEL_TYPE is deprecated; use OTEL_EXPORT_GRAFANA / OTEL_EXPORT_LANGFUSE / OTEL_EXPORT_LOCAL"
        );
    }

    if cfg.track_0_idle_mode && matches!(cfg.registry_adapter, RegistryAdapter::Grpc) {
        tracing::warn!(
            "EH_TRACK_0_IDLE_MODE=true with EH_REGISTRY_ADAPTER=grpc — external port \
             will be unbound, but the real Registry adapter is selected. Did you mean stub?"
        );
    }

    let pool = db::build_pool(&cfg).await?;
    db::run_migrations(&pool).await.unwrap_or_else(|err| {
        tracing::error!(?err, "migration run failed");
        std::process::exit(70);
    });

    let shutdown = Shutdown::default();

    // Spawn the LISTEN/NOTIFY fanout manager as a background task.
    // It uses a dedicated connection (not the main pool) so LISTEN registrations
    // are never silently dropped by pool recycling.
    let listen_manager =
        ListenNotifyManager::new(cfg.database_url.clone(), pool.clone(), metrics.clone());
    let listen_shutdown_rx = shutdown.shutdown_rx.clone();
    let _listen_handle = tokio::spawn(async move {
        listen_manager.run(listen_shutdown_rx).await;
    });

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
        otel_grafana = cfg.otel_export_grafana,
        otel_langfuse = cfg.otel_export_langfuse,
        otel_local = cfg.otel_export_local,
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
    telemetry::shutdown_telemetry();
    tracing::info!("exited cleanly");
    Ok(())
}
