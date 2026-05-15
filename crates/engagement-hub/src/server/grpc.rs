use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use tokio::sync::watch;
use tonic::transport::Server;
use tonic_health::server::HealthReporter;

pub struct GrpcServers {
    pub external_handle: Option<tokio::task::JoinHandle<Result<()>>>,
    pub internal_handle: tokio::task::JoinHandle<Result<()>>,
    pub external_reporter: Option<HealthReporter>,
    pub internal_reporter: HealthReporter,
}

pub async fn spawn(
    external_addr: SocketAddr,
    internal_addr: SocketAddr,
    bind_external: bool,
    shutdown: watch::Receiver<bool>,
) -> Result<GrpcServers> {
    // ---- internal server (always bound) ----
    let (mut internal_reporter, internal_health_svc) = tonic_health::server::health_reporter();
    internal_reporter
        .set_service_status("", tonic_health::ServingStatus::Serving)
        .await;

    let internal_shutdown = shutdown.clone();
    let internal_handle = tokio::spawn(async move {
        Server::builder()
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .http2_keepalive_interval(Some(Duration::from_secs(30)))
            .add_service(internal_health_svc)
            .serve_with_shutdown(internal_addr, async move {
                let mut s = internal_shutdown;
                let _ = s.wait_for(|v| *v).await;
            })
            .await
            .context("internal grpc server failed")?;
        Ok(())
    });

    // ---- external server (skipped in idle mode) ----
    let (external_reporter, external_handle) = if bind_external {
        let (mut rep, svc) = tonic_health::server::health_reporter();
        rep.set_service_status("", tonic_health::ServingStatus::Serving)
            .await;

        let mut ext_shutdown = shutdown.clone();
        let h = tokio::spawn(async move {
            Server::builder()
                .tcp_keepalive(Some(Duration::from_secs(30)))
                .http2_keepalive_interval(Some(Duration::from_secs(30)))
                .add_service(svc)
                .serve_with_shutdown(external_addr, async move {
                    let _ = ext_shutdown.wait_for(|v| *v).await;
                })
                .await
                .context("external grpc server failed")?;
            Ok(())
        });
        (Some(rep), Some(h))
    } else {
        tracing::warn!(
            %external_addr,
            "EH_TRACK_0_IDLE_MODE=true; not binding external gRPC port"
        );
        (None, None)
    };

    Ok(GrpcServers {
        external_handle,
        internal_handle,
        external_reporter,
        internal_reporter,
    })
}
