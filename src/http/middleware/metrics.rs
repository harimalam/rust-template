use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};

use crate::bootstrap::telemetry::{HTTP_REQUEST_DURATION, HTTP_REQUESTS_TOTAL};

/// Records request count and latency labelled by *route template* (e.g. `/api/v1/notes/{id}`),
/// never the raw path, so label cardinality stays bounded.
pub async fn track_http_metrics(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |p| p.as_str().to_owned());

    let response = next.run(req).await;

    let labels = [
        ("method", method),
        ("route", route),
        ("status", response.status().as_u16().to_string()),
    ];
    metrics::counter!(HTTP_REQUESTS_TOTAL, &labels).increment(1);
    metrics::histogram!(HTTP_REQUEST_DURATION, &labels).record(start.elapsed().as_secs_f64());
    response
}
