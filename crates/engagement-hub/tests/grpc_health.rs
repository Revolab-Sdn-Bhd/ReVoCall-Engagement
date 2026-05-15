use std::{net::SocketAddr, time::Duration};

use engagement_hub::server::grpc;
use tokio::sync::watch;
use tonic_health::pb::{
    HealthCheckRequest, health_check_response::ServingStatus, health_client::HealthClient,
};

async fn pick_port() -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a
}

async fn check(addr: SocketAddr) -> ServingStatus {
    let endpoint = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = HealthClient::new(endpoint);
    let resp = client
        .check(HealthCheckRequest {
            service: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    ServingStatus::try_from(resp.status).unwrap()
}

#[tokio::test]
async fn both_ports_serving_when_not_idle() {
    let ext = pick_port().await;
    let int = pick_port().await;
    let (shut_tx, shut_rx) = watch::channel(false);

    let servers = grpc::spawn(ext, int, true, shut_rx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;

    assert_eq!(check(ext).await, ServingStatus::Serving);
    assert_eq!(check(int).await, ServingStatus::Serving);

    shut_tx.send(true).unwrap();
    if let Some(h) = servers.external_handle {
        let _ = h.await;
    }
    let _ = servers.internal_handle.await;
}

#[tokio::test]
async fn external_port_unbound_in_idle_mode() {
    let ext = pick_port().await;
    let int = pick_port().await;
    let (shut_tx, shut_rx) = watch::channel(false);

    let servers = grpc::spawn(ext, int, false, shut_rx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;

    assert_eq!(check(int).await, ServingStatus::Serving);

    // External connect must fail — port not bound.
    let connect = tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(ext),
    )
    .await;
    assert!(
        connect.is_err() || connect.unwrap().is_err(),
        "external port should refuse connections"
    );

    assert!(servers.external_handle.is_none());

    shut_tx.send(true).unwrap();
    let _ = servers.internal_handle.await;
}
