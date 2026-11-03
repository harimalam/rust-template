use anyhow::Context;
use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::config::DatabaseSettings;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn connect(settings: &DatabaseSettings) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .min_connections(settings.min_connections)
        .acquire_timeout(settings.acquire_timeout)
        .connect(&settings.url)
        .await
        .context("failed to connect to PostgreSQL")
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .context("failed to run database migrations")
}
