pub mod dto;
pub mod handlers;
pub mod repository;
pub mod service;
pub mod sniff;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};

use crate::bootstrap::state::AppState;

/// Mounted at `/api/v1/files`.
pub fn routes(state: &AppState) -> Router<AppState> {
    // Multipart envelope overhead on top of the file itself.
    let upload_limit = state.settings.s3.max_upload_bytes + 64 * 1024;
    Router::new()
        .route(
            "/",
            post(handlers::upload)
                .layer(DefaultBodyLimit::max(upload_limit))
                .get(handlers::list),
        )
        .route("/{id}/download-url", get(handlers::download_url))
        .route("/{id}", axum::routing::delete(handlers::delete))
}
