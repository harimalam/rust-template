//! HTTP layer for authentication: extract, validate, call the service, shape the response.

use axum::{
    extract::State,
    http::{HeaderMap, header::USER_AGENT},
};

use super::{
    dto::{
        EmailRequest, LoginRequest, RefreshRequest, RegisterRequest, ResetPasswordRequest,
        TokenPair, TokenRequest, UserDto,
    },
    service::{self, ClientContext},
};
use crate::{
    bootstrap::state::AppState,
    http::{
        AppResult,
        error::ErrorBody,
        extractors::{ClientIp, ValidatedJson},
        response::{ApiResponse, Envelope, Message, created, message, ok},
    },
};

fn context(ip: ClientIp, headers: &HeaderMap) -> ClientContext {
    ClientContext {
        ip: Some(ip.0.to_string()),
        user_agent: headers
            .get(USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.chars().take(256).collect()),
    }
}

/// Create an account and queue a verification email.
#[utoipa::path(
    post, path = "/api/v1/auth/register", tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "Account created", body = Envelope<UserDto>),
        (status = 409, description = "Email already registered", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody),
        (status = 429, description = "Rate limited", body = ErrorBody),
    )
)]
pub async fn register(
    State(state): State<AppState>,
    ValidatedJson(req): ValidatedJson<RegisterRequest>,
) -> AppResult<ApiResponse<UserDto>> {
    Ok(created(service::register(&state, req).await?))
}

/// Exchange credentials for an access + refresh token pair.
#[utoipa::path(
    post, path = "/api/v1/auth/login", tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Authenticated", body = Envelope<TokenPair>),
        (status = 401, description = "Invalid credentials", body = ErrorBody),
        (status = 403, description = "Email not verified or account disabled", body = ErrorBody),
        (status = 429, description = "Rate limited or temporarily locked out", body = ErrorBody),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    ValidatedJson(req): ValidatedJson<LoginRequest>,
) -> AppResult<ApiResponse<TokenPair>> {
    Ok(ok(
        service::login(&state, req, &context(ip, &headers)).await?
    ))
}

/// Rotate a refresh token. The old token stops working; reusing it revokes the whole session.
#[utoipa::path(
    post, path = "/api/v1/auth/refresh", tag = "auth",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "New token pair", body = Envelope<TokenPair>),
        (status = 401, description = "Invalid, expired or reused token", body = ErrorBody),
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    ValidatedJson(req): ValidatedJson<RefreshRequest>,
) -> AppResult<ApiResponse<TokenPair>> {
    Ok(ok(service::refresh(
        &state,
        &req.refresh_token,
        &context(ip, &headers),
    )
    .await?))
}

/// Revoke the session that owns this refresh token.
#[utoipa::path(
    post, path = "/api/v1/auth/logout", tag = "auth",
    request_body = RefreshRequest,
    responses((status = 200, description = "Logged out", body = Envelope<Message>))
)]
pub async fn logout(
    State(state): State<AppState>,
    ValidatedJson(req): ValidatedJson<RefreshRequest>,
) -> AppResult<ApiResponse<Message>> {
    service::logout(&state, &req.refresh_token).await?;
    Ok(message("logged out"))
}

/// Confirm an email address using the token from the verification email.
#[utoipa::path(
    post, path = "/api/v1/auth/verify-email", tag = "auth",
    request_body = TokenRequest,
    responses(
        (status = 200, description = "Email verified", body = Envelope<Message>),
        (status = 400, description = "Invalid or expired token", body = ErrorBody),
    )
)]
pub async fn verify_email(
    State(state): State<AppState>,
    ValidatedJson(req): ValidatedJson<TokenRequest>,
) -> AppResult<ApiResponse<Message>> {
    service::verify_email(&state, &req.token).await?;
    Ok(message("email verified"))
}

/// Send another verification email. Always responds 200 to avoid revealing which emails exist.
#[utoipa::path(
    post, path = "/api/v1/auth/resend-verification", tag = "auth",
    request_body = EmailRequest,
    responses((status = 200, description = "Accepted", body = Envelope<Message>))
)]
pub async fn resend_verification(
    State(state): State<AppState>,
    ValidatedJson(req): ValidatedJson<EmailRequest>,
) -> AppResult<ApiResponse<Message>> {
    service::resend_verification(&state, &req.email).await?;
    Ok(message(
        "if the account exists and is unverified, a new email has been sent",
    ))
}

/// Start a password reset. Always responds 200 to avoid revealing which emails exist.
#[utoipa::path(
    post, path = "/api/v1/auth/forgot-password", tag = "auth",
    request_body = EmailRequest,
    responses((status = 200, description = "Accepted", body = Envelope<Message>))
)]
pub async fn forgot_password(
    State(state): State<AppState>,
    ValidatedJson(req): ValidatedJson<EmailRequest>,
) -> AppResult<ApiResponse<Message>> {
    service::forgot_password(&state, &req.email).await?;
    Ok(message(
        "if the account exists, a password reset email has been sent",
    ))
}

/// Set a new password using a reset token. Ends all existing sessions.
#[utoipa::path(
    post, path = "/api/v1/auth/reset-password", tag = "auth",
    request_body = ResetPasswordRequest,
    responses(
        (status = 200, description = "Password changed", body = Envelope<Message>),
        (status = 400, description = "Invalid or expired token", body = ErrorBody),
    )
)]
pub async fn reset_password(
    State(state): State<AppState>,
    ValidatedJson(req): ValidatedJson<ResetPasswordRequest>,
) -> AppResult<ApiResponse<Message>> {
    service::reset_password(&state, req).await?;
    Ok(message("password has been reset"))
}
