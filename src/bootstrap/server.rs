use std::{future::IntoFuture, net::SocketAddr, sync::Arc};

use anyhow::Context;
use metrics_exporter_prometheus::PrometheusHandle;
use tokio::{net::TcpListener, sync::Notify};

use super::{router::build_router, shutdown::shutdown_signal, state::AppState};
use crate::config::Settings;

/// Runs the HTTP API until SIGTERM / Ctrl-C, then drains in-flight requests (bounded by
/// `SHUTDOWN_TIMEOUT_SECS`) and closes the connection pool.
pub async fn serve(settings: Settings, metrics: Option<PrometheusHandle>) -> anyhow::Result<()> {
    let addr = format!("{}:{}", settings.http.host, settings.http.port);
    let shutdown_timeout = settings.http.shutdown_timeout;
    let env = settings.env;

    let state = AppState::connect(settings, metrics).await?;
    let app = build_router(state.clone());
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("cannot bind {addr}"))?;
    tracing::info!(%addr, ?env, "API listening");

    let begin_drain = Arc::new(Notify::new());
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown({
        let begin_drain = begin_drain.clone();
        async move { begin_drain.notified().await }
    });
    let mut server = std::pin::pin!(server.into_future());

    tokio::select! {
        result = &mut server => result.context("server error")?,
        () = async {
            shutdown_signal().await;
            begin_drain.notify_one();
            tokio::time::sleep(shutdown_timeout).await;
        } => tracing::warn!("graceful shutdown timed out; exiting with requests still open"),
    }

    state.db.close().await;
    tracing::info!("API stopped");
    Ok(())
}
