use std::{net::SocketAddr, time::Duration};

use assert_cmd::cargo::CommandCargoExt;
use sqlx::postgres::PgPoolOptions;

fn db_url() -> String {
    std::env::var("EH_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:eh_test@localhost:5432/engagement_hub_db".into())
}

async fn pick_port() -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    a
}

async fn wait_http(addr: SocketAddr, path: &str) -> reqwest::Response {
    let url = format!("http://{addr}{path}");
    for _ in 0..50 {
        if let Ok(r) = reqwest::get(&url).await {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("http server never came up at {url}");
}

#[tokio::test]
async fn invalid_prod_stub_exits_78() {
    let mut cmd = std::process::Command::cargo_bin("engagement-hub").unwrap();
    cmd.env("EH_ENV", "production")
        .env("EH_REGISTRY_ADAPTER", "stub")
        .env("EH_TRACK_0_IDLE_MODE", "false")
        .env("EH_DATABASE_URL", "postgres://x")
        .env("EH_LOG_FORMAT", "pretty");
    let output = cmd.output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(78),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("EH_REGISTRY_ADAPTER=stub is forbidden in production"),
        "stderr was: {stderr}"
    );
}

#[tokio::test]
async fn normal_start_serves_all_three_ports() {
    let ext = pick_port().await;
    let int = pick_port().await;
    let http = pick_port().await;

    let child = std::process::Command::cargo_bin("engagement-hub")
        .unwrap()
        .env("EH_ENV", "dev")
        .env("EH_REGISTRY_ADAPTER", "stub")
        .env("EH_TRACK_0_IDLE_MODE", "false")
        .env("EH_DATABASE_URL", db_url())
        .env("EH_EXTERNAL_GRPC_ADDR", ext.to_string())
        .env("EH_INTERNAL_GRPC_ADDR", int.to_string())
        .env("EH_HTTP_ADDR", http.to_string())
        .spawn()
        .unwrap();
    let child = scopeguard::guard(child, |mut c| {
        let _ = c.kill();
    });

    let livez = wait_http(http, "/livez").await;
    assert_eq!(livez.status(), 200);

    let metrics = wait_http(http, "/metrics").await.text().await.unwrap();
    assert!(
        metrics.contains(r#"engagementhub_registry_adapter_kind{kind="stub"} 1"#),
        "metrics: {metrics}"
    );

    // gRPC health on both ports
    for addr in [ext, int] {
        let endpoint = tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = tonic_health::pb::health_client::HealthClient::new(endpoint);
        let resp = client
            .check(tonic_health::pb::HealthCheckRequest {
                service: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            resp.status,
            tonic_health::pb::health_check_response::ServingStatus::Serving as i32,
            "addr {addr} not serving"
        );
    }

    drop(child); // scopeguard kills
}

#[tokio::test]
async fn idle_mode_refuses_external_port() {
    let ext = pick_port().await;
    let int = pick_port().await;
    let http = pick_port().await;

    let child = std::process::Command::cargo_bin("engagement-hub")
        .unwrap()
        .env("EH_ENV", "production")
        .env("EH_REGISTRY_ADAPTER", "stub")
        .env("EH_TRACK_0_IDLE_MODE", "true")
        .env("EH_DATABASE_URL", db_url())
        .env("EH_EXTERNAL_GRPC_ADDR", ext.to_string())
        .env("EH_INTERNAL_GRPC_ADDR", int.to_string())
        .env("EH_HTTP_ADDR", http.to_string())
        .spawn()
        .unwrap();
    let child = scopeguard::guard(child, |mut c| {
        let _ = c.kill();
    });

    let _ = wait_http(http, "/livez").await;

    // External port should refuse
    let connect = tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(ext),
    )
    .await;
    assert!(
        matches!(connect, Err(_) | Ok(Err(_))),
        "external port should be unbound"
    );

    // Internal still serves
    let endpoint = tonic::transport::Channel::from_shared(format!("http://{int}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = tonic_health::pb::health_client::HealthClient::new(endpoint);
    let resp = client
        .check(tonic_health::pb::HealthCheckRequest {
            service: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        tonic_health::pb::health_check_response::ServingStatus::Serving as i32
    );

    drop(child);
}

#[tokio::test]
async fn migration_up_down_clean() {
    // After at least one prior test ran migrations, all 5 tables exist.
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    // Apply migrations explicitly (idempotent)
    engagement_hub::db::MIGRATOR.run(&pool).await.unwrap();

    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' \
           AND table_name IN ('engagements','engagement_invocations','route_resolutions','engagement_events','engagement_audit') \
         ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(tables.len(), 5, "got {tables:?}");

    // Run the .down.sql
    let down_sql = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260515000000_initial_schema.down.sql"
    ))
    .unwrap();
    sqlx::raw_sql(&down_sql).execute(&pool).await.unwrap();

    let count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables \
         WHERE table_schema='public' \
           AND table_name IN ('engagements','engagement_invocations','route_resolutions','engagement_events','engagement_audit')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 0);

    // Migrator should re-apply cleanly. sqlx tracks state in _sqlx_migrations
    // which we also need to wipe.
    sqlx::query("DROP TABLE IF EXISTS _sqlx_migrations")
        .execute(&pool)
        .await
        .unwrap();
    engagement_hub::db::MIGRATOR.run(&pool).await.unwrap();

    let count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables \
         WHERE table_schema='public' \
           AND table_name IN ('engagements','engagement_invocations','route_resolutions','engagement_events','engagement_audit')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 5);
}
