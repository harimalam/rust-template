//! Centralised error type. Every handler returns `AppResult<T>`; this is the only place
//! that decides status codes and the public JSON error shape.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

use super::request_context::current_request_id;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("too many requests")]
    TooManyRequests { retry_after_secs: u64 },
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("{0}")]
    UnsupportedMediaType(String),
    #[error("{0}")]
    ServiceUnavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn internal(err: impl Into<anyhow::Error>) -> Self {
        Self::Internal(err.into())
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation_failed",
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::TooManyRequests { .. } => "rate_limited",
            Self::PayloadTooLarge => "payload_too_large",
            Self::UnsupportedMediaType(_) => "unsupported_media_type",
            Self::ServiceUnavailable(_) => "service_unavailable",
            Self::Internal(_) => "internal_error",
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<FieldError>>,
    pub request_id: Option<String>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if let Self::Internal(err) = &self {
            // Full detail goes to logs only; clients get a generic message.
            tracing::error!(error = ?err, "internal error");
        }
        let message = match &self {
            Self::Internal(_) => "an internal error occurred".to_owned(),
            other => other.to_string(),
        };
        let details = match &self {
            Self::Validation(fields) => Some(fields.clone()),
            _ => None,
        };
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code(),
                message,
                details,
                request_id: current_request_id(),
            },
        };

        let mut response = (status, Json(body)).into_response();
        if let Self::TooManyRequests { retry_after_secs } = self
            && let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, v);
        }
        response
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Self::Conflict("resource already exists".into())
            }
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
                Self::BadRequest("referenced resource does not exist".into())
            }
            _ => Self::Internal(err.into()),
        }
    }
}

impl From<validator::ValidationErrors> for AppError {
    fn from(errors: validator::ValidationErrors) -> Self {
        let mut fields: Vec<FieldError> = errors
            .field_errors()
            .into_iter()
            .flat_map(|(field, errs)| {
                errs.iter().map(move |e| FieldError {
                    field: field.to_string(),
                    message: e
                        .message
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("invalid value ({})", e.code)),
                })
            })
            .collect();
        fields.sort_by(|a, b| a.field.cmp(&b.field));
        Self::Validation(fields)
    }
}

impl From<redis::RedisError> for AppError {
    fn from(err: redis::RedisError) -> Self {
        Self::Internal(err.into())
    }
}
