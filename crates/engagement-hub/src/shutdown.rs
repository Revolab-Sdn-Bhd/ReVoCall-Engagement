use tokio::signal;
use tokio::sync::watch;

pub struct Shutdown {
    pub drain_tx: watch::Sender<bool>,
    pub drain_rx: watch::Receiver<bool>,
    pub shutdown_tx: watch::Sender<bool>,
    pub shutdown_rx: watch::Receiver<bool>,
}

impl Default for Shutdown {
    fn default() -> Self {
        let (drain_tx, drain_rx) = watch::channel(false);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            drain_tx,
            drain_rx,
            shutdown_tx,
            shutdown_rx,
        }
    }
}

/// Resolve once SIGTERM or SIGINT is received.
pub async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let mut intr = signal(SignalKind::interrupt()).expect("install SIGINT handler");
        tokio::select! {
            _ = term.recv() => tracing::info!("received SIGTERM"),
            _ = intr.recv() => tracing::info!("received SIGINT"),
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("received Ctrl-C");
    }
}
