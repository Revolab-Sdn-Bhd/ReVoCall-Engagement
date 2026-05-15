use std::{net::SocketAddr, sync::Arc, time::Duration};

use engagement_hub::{
    config::{Env, RegistryAdapter},
    metrics::Metrics,
    server::http,
};
use tokio::sync::watch;

async fn bind_test_addr() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

#[tokio::test]
async fn livez_returns_ok() {
    let addr = bind_test_addr().await;
    let metrics = Arc::new(Metrics::new(RegistryAdapter::Stub, Env::Dev, false).unwrap());
    let (drain_tx, drain_rx) = watch::channel(false);
    let (shut_tx, shut_rx) = watch::channel(false);

    let server = tokio::spawn(http::serve(addr, metrics.clone(), drain_rx, shut_rx));
    tokio::time::sleep(Duration::from_millis(150)).await;

    let resp = reqwest::get(format!("http://{addr}/livez")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");

    let metrics_text = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        metrics_text.contains(
            r#"engagementhub_registry_adapter_kind{env="dev",idle_mode="false",kind="stub"} 1"#
        ),
        "missing active=stub metric line in:\n{metrics_text}"
    );

    // /readyz starts as 200, flips to 503 once draining
    let r = reqwest::get(format!("http://{addr}/readyz")).await.unwrap();
    assert_eq!(r.status(), 200);
    drain_tx.send(true).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let r = reqwest::get(format!("http://{addr}/readyz")).await.unwrap();
    assert_eq!(r.status(), 503);

    shut_tx.send(true).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
    let _ = drain_tx;
}
