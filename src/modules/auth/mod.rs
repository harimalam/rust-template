pub mod dto;
pub mod emails;
pub mod handlers;
pub mod jwt;
pub mod password;
pub mod rbac;
pub mod repository;
pub mod service;
pub mod tokens;

use axum::{Router, middleware, routing::post};

use crate::{bootstrap::state::AppState, http::middleware::rate_limit::limit_auth};

/// Mounted at `/api/v1/auth`. Every route is IP rate limited.
pub fn routes(state: &AppState) -> Router<AppState> {
    Router::new()
        .route("/register", post(handlers::register))
        .route("/login", post(handlers::login))
        .route("/refresh", post(handlers::refresh))
        .route("/logout", post(handlers::logout))
        .route("/verify-email", post(handlers::verify_email))
        .route("/resend-verification", post(handlers::resend_verification))
        .route("/forgot-password", post(handlers::forgot_password))
        .route("/reset-password", post(handlers::reset_password))
        .layer(middleware::from_fn_with_state(state.clone(), limit_auth))
}
