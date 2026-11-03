//! One-shot / auxiliary commands: worker, migrate, create-admin, healthcheck.

use std::{sync::Arc, time::Duration};

use anyhow::Context;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::{
    bootstrap::shutdown::shutdown_signal,
    config::Settings,
    infra::{db, mail::SmtpMailer, queue::consumer::run_worker},
    jobs::handler::AppJobHandler,
    modules::auth::service,
};

pub async fn worker(settings: Settings) -> anyhow::Result<()> {
    let mailer = SmtpMailer::new(&settings.smtp)?;
    let handler = Arc::new(AppJobHandler {
        mailer: Arc::new(mailer),
    });
    run_worker(
        &settings.queue,
        handler,
        shutdown_signal(),
        settings.http.shutdown_timeout,
    )
    .await
}

pub async fn migrate(settings: Settings) -> anyhow::Result<()> {
    let pool = db::connect(&settings.database).await?;
    db::migrate(&pool).await?;
    pool.close().await;
    tracing::info!("migrations applied");
    Ok(())
}

pub async fn create_admin(
    settings: Settings,
    email: String,
    password: String,
    name: String,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        password.len() >= 12,
        "admin password must be at least 12 characters"
    );
    let pool = db::connect(&settings.database).await?;
    let user = service::create_admin(&pool, &email, password, &name)
        .await
        .map_err(|e| anyhow::anyhow!("could not create admin: {e}"))?;
    pool.close().await;
    println!("created admin {} ({})", user.email, user.id);
    Ok(())
}

/// Minimal HTTP probe for container HEALTHCHECK (the runtime image has no curl).
/// Exits non-zero unless `/health/live` answers 200.
pub async fn healthcheck() -> anyhow::Result<()> {
    let port: u16 = std::env::var("HTTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let probe = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream
            .write_all(b"GET /health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut head = [0u8; 32];
        let n = stream.read(&mut head).await?;
        anyhow::Ok(String::from_utf8_lossy(&head[..n]).into_owned())
    };
    let response = tokio::time::timeout(Duration::from_secs(3), probe)
        .await
        .context("health probe timed out")??;
    anyhow::ensure!(
        response.starts_with("HTTP/1.1 200"),
        "unhealthy: {response:?}"
    );
    Ok(())
}
