use engagement_hub::config::{Config, ConfigError, Env, RegistryAdapter};

fn base() -> Config {
    Config {
        env: Env::Production,
        registry_adapter: RegistryAdapter::Stub,
        track_0_idle_mode: false,
        database_url: "postgres://x".into(),
        external_grpc_addr: "0.0.0.0:8443".parse().unwrap(),
        internal_grpc_addr: "0.0.0.0:8444".parse().unwrap(),
        http_addr: "0.0.0.0:9090".parse().unwrap(),
        db_pool_min: 10,
        db_pool_max: 25,
        db_idle_timeout_secs: 300,
        db_statement_timeout_ms: 5000,
        db_slow_query_ms: 500,
        log_format: "json".into(),
    }
}

#[test]
fn prod_stub_idle_true_ok() {
    let mut c = base();
    c.track_0_idle_mode = true;
    c.validate().expect("must accept prod+stub+idle=true");
}

#[test]
fn prod_stub_idle_false_rejected() {
    let c = base(); // idle=false
    match c.validate() {
        Err(ConfigError::ProdStubWithoutIdle) => {}
        other => panic!("expected ProdStubWithoutIdle, got {other:?}"),
    }
}

#[test]
fn prod_grpc_ok() {
    let mut c = base();
    c.registry_adapter = RegistryAdapter::Grpc;
    c.validate().expect("must accept prod+grpc");
}

#[test]
fn dev_stub_ok() {
    let mut c = base();
    c.env = Env::Dev;
    c.validate().expect("must accept dev+stub");
}

#[test]
fn zero_statement_timeout_rejected() {
    let mut c = base();
    // Use dev env so we don't hit ProdStubWithoutIdle first
    c.env = Env::Dev;
    c.db_statement_timeout_ms = 0;
    assert!(matches!(
        c.validate(),
        Err(ConfigError::StatementTimeoutDisabled)
    ));
}
