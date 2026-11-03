pub mod dto;
pub mod handlers;
pub mod repository;
pub mod service;

use axum::{
    Router,
    routing::{get, put},
};

use crate::bootstrap::state::AppState;

/// Mounted at `/api/v1/users`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(handlers::me).patch(handlers::update_me))
        .route("/", get(handlers::list))
        .route("/{id}/roles", put(handlers::set_roles))
}
