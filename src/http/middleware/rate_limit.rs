//! Fixed-window rate limiting backed by Valkey, so limits hold across all API replicas.
//! If Valkey is unreachable the request is allowed (fail-open) and the error is logged:
//! an outage of the cache must not take authentication down with it.

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{
    bootstrap::state::AppState,
    http::{AppError, extractors::ClientIp},
};

pub async fn limit_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let (parts, body) = req.into_parts();
    let ip = ClientIp::resolve(&parts, state.settings.http.trust_proxy_headers)
        .map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
    let key = format!("rl:auth:{}:{ip}", parts.uri.path());
    let limits = &state.settings.rate_limit;

    match state.cache.hit_window(&key, limits.auth_window).await {
        Ok(hit) if hit.count > u64::from(limits.auth_max) => {
            metrics::counter!("rate_limited_requests_total", "scope" => "auth").increment(1);
            AppError::TooManyRequests {
                retry_after_secs: hit.retry_after_secs,
            }
            .into_response()
        }
        Ok(_) => next.run(Request::from_parts(parts, body)).await,
        Err(err) => {
            tracing::warn!(error = ?err, "rate limiter unavailable; allowing request");
            next.run(Request::from_parts(parts, body)).await
        }
    }
}
