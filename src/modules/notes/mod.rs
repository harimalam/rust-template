//! Example CRUD module. Copy this folder to start a new feature:
//! `domain` (entity) → `dto` (wire types) → `repository` (SQL) → `service` (rules, cache) →
//! `handlers` (HTTP) → `routes()`.

pub mod domain;
pub mod dto;
pub mod handlers;
pub mod repository;
pub mod service;

use axum::{Router, routing::get};

use crate::bootstrap::state::AppState;

/// Mounted at `/api/v1/notes`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list).post(handlers::create))
        .route(
            "/{id}",
            get(handlers::get)
                .patch(handlers::update)
                .delete(handlers::delete),
        )
}
