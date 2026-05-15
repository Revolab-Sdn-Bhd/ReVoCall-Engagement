use std::{str::FromStr, time::Duration};

use anyhow::{Context, Result};
use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgPool, PgPoolOptions},
};
use tracing::log::LevelFilter;

use crate::config::Config;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub async fn build_pool(cfg: &Config) -> Result<PgPool> {
    let statement_timeout_ms = cfg.db_statement_timeout_ms;
    let slow_query = Duration::from_millis(cfg.db_slow_query_ms);

    let mut connect_options = PgConnectOptions::from_str(&cfg.database_url)
        .context("EH_DATABASE_URL must be a valid Postgres URL")?
        .application_name("engagement-hub");

    connect_options = connect_options
        .log_statements(LevelFilter::Debug)
        .log_slow_statements(LevelFilter::Warn, slow_query);

    let pool = PgPoolOptions::new()
        .min_connections(cfg.db_pool_min)
        .max_connections(cfg.db_pool_max)
        .idle_timeout(Duration::from_secs(cfg.db_idle_timeout_secs))
        .after_connect(move |conn, _meta| {
            let sql = format!("SET statement_timeout = {statement_timeout_ms}");
            Box::pin(async move {
                sqlx::query(&sql).execute(&mut *conn).await?;
                Ok(())
            })
        })
        .connect_with(connect_options)
        .await
        .context("failed to open Postgres pool")?;

    Ok(pool)
}

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .context("migration failed; refusing to start")
}
