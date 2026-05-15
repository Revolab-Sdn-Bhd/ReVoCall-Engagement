use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::metrics::Metrics;

#[derive(Clone)]
struct HttpState {
    metrics: Arc<Metrics>,
    draining: watch::Receiver<bool>,
}

pub async fn serve(
    addr: SocketAddr,
    metrics: Arc<Metrics>,
    draining: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let state = HttpState { metrics, draining };

    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/livez", get(livez_handler))
        .route("/readyz", get(readyz_handler))
        .with_state(state);

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind http {addr}"))?;
    tracing::info!(%addr, "http server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|s| *s).await;
        })
        .await
        .context("http server failed")
}

async fn metrics_handler(State(state): State<HttpState>) -> impl IntoResponse {
    match state.metrics.gather_text() {
        Ok(text) => (StatusCode::OK, text),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("gather failed: {e}"),
        ),
    }
}

async fn livez_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn readyz_handler(State(state): State<HttpState>) -> impl IntoResponse {
    if *state.draining.borrow() {
        (StatusCode::SERVICE_UNAVAILABLE, "draining")
    } else {
        (StatusCode::OK, "ready")
    }
}
