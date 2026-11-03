use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::{LogFormat, Settings};

pub const HTTP_REQUESTS_TOTAL: &str = "http_requests_total";
pub const HTTP_REQUEST_DURATION: &str = "http_request_duration_seconds";

/// Structured logging: pretty for local development, one JSON object per line elsewhere.
pub fn init_tracing(settings: &Settings) {
    let filter = EnvFilter::try_new(&settings.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter);
    match settings.log_format {
        LogFormat::Json => registry
            .with(
                fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true),
            )
            .init(),
        LogFormat::Pretty => registry.with(fmt::layer().pretty()).init(),
    }
}

/// Installs the global Prometheus recorder. Call once per process.
pub fn init_metrics() -> anyhow::Result<PrometheusHandle> {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(HTTP_REQUEST_DURATION.to_owned()),
            &[
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ],
        )?
        .install_recorder()?;
    Ok(handle)
}
