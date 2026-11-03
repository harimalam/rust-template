use std::net::{IpAddr, SocketAddr};

use axum::{
    Json,
    extract::{
        ConnectInfo, FromRequest, FromRequestParts, Request,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{StatusCode, request::Parts},
};
use serde::de::DeserializeOwned;
use validator::Validate;

use super::AppError;
use crate::bootstrap::state::AppState;

/// JSON body that is deserialized **and** validated; failures become standard error responses.
pub struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|rej| match rej {
                JsonRejection::BytesRejection(inner)
                    if inner.status() == StatusCode::PAYLOAD_TOO_LARGE =>
                {
                    AppError::PayloadTooLarge
                }
                JsonRejection::MissingJsonContentType(_) => {
                    AppError::UnsupportedMediaType("expected Content-Type: application/json".into())
                }
                other => AppError::BadRequest(format!("invalid JSON body: {}", other.body_text())),
            })?;
        value.validate()?;
        Ok(Self(value))
    }
}

/// The caller's IP. Uses the socket address, or — only when `TRUST_PROXY_HEADERS=true` — the
/// last `X-Forwarded-For` entry (the hop appended by our own trusted reverse proxy; earlier
/// entries are client-supplied and spoofable).
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

impl ClientIp {
    pub fn resolve(parts: &Parts, trust_proxy: bool) -> Option<IpAddr> {
        if trust_proxy {
            let forwarded = parts
                .headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
                .and_then(|v| v.trim().parse::<IpAddr>().ok());
            if forwarded.is_some() {
                return forwarded;
            }
        }
        parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip())
    }
}

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(
            Self::resolve(parts, state.settings.http.trust_proxy_headers)
                .unwrap_or(IpAddr::from([0, 0, 0, 0])),
        ))
    }
}

/// `Path<T>` whose rejection uses the standard JSON error shape.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub struct ApiPath<T>(pub T);

/// `Query<T>` whose rejection uses the standard JSON error shape.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppError))]
pub struct ApiQuery<T>(pub T);

impl From<PathRejection> for AppError {
    fn from(rejection: PathRejection) -> Self {
        Self::BadRequest(format!("invalid path parameter: {}", rejection.body_text()))
    }
}

impl From<QueryRejection> for AppError {
    fn from(rejection: QueryRejection) -> Self {
        Self::BadRequest(format!(
            "invalid query parameter: {}",
            rejection.body_text()
        ))
    }
}
