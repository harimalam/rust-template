use std::any::Any;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

use super::{openapi::ApiDoc, state::AppState};
use crate::{
    config::Settings,
    http::{
        AppError,
        error::{ErrorBody, ErrorDetail},
        middleware::metrics::track_http_metrics,
        request_context::{RequestId, current_request_id, request_id_middleware},
    },
    modules::{auth, files, health, notes, users},
};

pub fn build_router(state: AppState) -> Router {
    let settings = state.settings.clone();

    let api = Router::new()
        .nest("/auth", auth::routes(&state))
        .nest("/users", users::routes())
        .nest("/notes", notes::routes())
        .nest("/files", files::routes(&state));

    let mut app = Router::new()
        .nest("/api/v1", api)
        .nest("/health", health::routes());

    if let Some(handle) = state.metrics.clone() {
        // Restrict /metrics to your monitoring network at the reverse proxy / ingress.
        app = app.route("/metrics", get(move || async move { handle.render() }));
    }

    let mut app = app.fallback(not_found).with_state(state);

    if settings.http.docs_enabled {
        // The docs UI loads a script from a CDN, so it gets a looser CSP than the API.
        let docs: Router = Router::new().merge(Scalar::with_url("/docs", ApiDoc::openapi())).layer(
            SetResponseHeaderLayer::overriding(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(
                    "default-src 'self'; script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
                     style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
                     font-src 'self' data: https://fonts.gstatic.com https://fonts.scalar.com; img-src 'self' data: https:; \
                     connect-src 'self'",
                ),
            ),
        );
        app = app.merge(docs);
    }

    if settings.env.is_production() {
        app = app.layer(header_if_absent(
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=63072000; includeSubDomains",
        ));
    }

    // Layers are applied bottom-up: the last one added is the outermost.
    app = app
        .layer(DefaultBodyLimit::max(settings.http.max_body_bytes))
        .layer(header_if_absent(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header_if_absent(header::X_FRAME_OPTIONS, "DENY"))
        .layer(header_if_absent(header::REFERRER_POLICY, "no-referrer"))
        .layer(header_if_absent(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; frame-ancestors 'none'",
        ))
        .layer(cors(&settings))
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            settings.http.request_timeout,
        ))
        .layer(middleware::from_fn(track_http_metrics))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &Request| {
                    let request_id = req
                        .extensions()
                        .get::<RequestId>()
                        .map_or("-", |id| id.0.as_str())
                        .to_owned();
                    tracing::info_span!(
                        "http",
                        method = %req.method(),
                        path = %req.uri().path(),
                        %request_id,
                    )
                })
                .on_response(DefaultOnResponse::new().level(tracing::Level::INFO)),
        )
        .layer(CatchPanicLayer::custom(handle_panic))
        .layer(middleware::from_fn(request_id_middleware));
    app
}

/// Sets a response header unless a handler already set it (lets the docs page relax the CSP).
fn header_if_absent(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

async fn not_found() -> AppError {
    AppError::NotFound("route")
}

fn handle_panic(_: Box<dyn Any + Send + 'static>) -> Response {
    tracing::error!("handler panicked");
    let body = ErrorBody {
        error: ErrorDetail {
            code: "internal_error",
            message: "an internal error occurred".into(),
            details: None,
            request_id: current_request_id(),
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

fn cors(settings: &Settings) -> CorsLayer {
    let origins = &settings.http.cors_allowed_origins;
    let allow_origin = if origins.iter().any(|o| o == "*") {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()))
    };
    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-request-id"),
        ])
        .expose_headers([HeaderName::from_static("x-request-id"), header::RETRY_AFTER])
        .max_age(std::time::Duration::from_secs(3600))
}
