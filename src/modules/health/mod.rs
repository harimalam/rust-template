use std::{collections::BTreeMap, future::Future, time::Duration};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Serialize;
use utoipa::ToSchema;

use crate::bootstrap::state::AppState;

const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/live", get(live))
        .route("/ready", get(ready))
}

#[derive(Serialize, ToSchema)]
pub struct LiveReport {
    pub status: &'static str,
}

#[derive(Serialize, ToSchema)]
pub struct ReadyReport {
    pub status: &'static str,
    pub checks: BTreeMap<&'static str, &'static str>,
}

/// Liveness: the process is up. Never touches dependencies, so a dependency outage doesn't
/// cause the orchestrator to restart a healthy process.
#[utoipa::path(get, path = "/health/live", tag = "health", responses((status = 200, body = LiveReport)))]
pub async fn live() -> Json<LiveReport> {
    Json(LiveReport { status: "ok" })
}

async fn check<F>(name: &'static str, fut: F) -> (&'static str, &'static str)
where
    F: Future<Output = anyhow::Result<()>>,
{
    match tokio::time::timeout(CHECK_TIMEOUT, fut).await {
        Ok(Ok(())) => (name, "ok"),
        Ok(Err(err)) => {
            tracing::warn!(error = ?err, check = name, "readiness check failed");
            (name, "unavailable")
        }
        Err(_) => {
            tracing::warn!(check = name, "readiness check timed out");
            (name, "timeout")
        }
    }
}

/// Readiness: every dependency this instance needs to serve traffic is reachable.
#[utoipa::path(
    get, path = "/health/ready", tag = "health",
    responses((status = 200, body = ReadyReport), (status = 503, body = ReadyReport))
)]
pub async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadyReport>) {
    let (database, cache, storage, queue) = tokio::join!(
        check("database", async {
            sqlx::query("SELECT 1").execute(&state.db).await?;
            Ok(())
        }),
        check("cache", state.cache.ping()),
        check("storage", state.storage.ping()),
        check("queue", state.jobs.ping()),
    );
    let checks: BTreeMap<_, _> = [database, cache, storage, queue].into_iter().collect();
    let healthy = checks.values().all(|v| *v == "ok");
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(ReadyReport {
            status: if healthy { "ok" } else { "degraded" },
            checks,
        }),
    )
}
