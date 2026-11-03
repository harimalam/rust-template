//! Request-ID / correlation-ID propagation.
//!
//! The ID is taken from an incoming `x-request-id` header (so it can be correlated
//! across services) or generated, then exposed through a task-local so that error
//! responses and background-job publishers can attach it without extra plumbing.

use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use uuid::Uuid;

pub const REQUEST_ID_HEADER: &str = "x-request-id";

tokio::task_local! {
    static REQUEST_ID: String;
}

/// The current request's ID, if called from within a request.
pub fn current_request_id() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

#[derive(Clone, Debug)]
pub struct RequestId(pub String);

fn is_acceptable(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| is_acceptable(v))
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    req.extensions_mut().insert(RequestId(id.clone()));
    let mut response = REQUEST_ID.scope(id.clone(), next.run(req)).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::is_acceptable;

    #[test]
    fn rejects_unsafe_ids() {
        assert!(is_acceptable("abc-123_x.y"));
        assert!(!is_acceptable(""));
        assert!(!is_acceptable("has space"));
        assert!(!is_acceptable("line\nbreak"));
        assert!(!is_acceptable(&"a".repeat(129)));
    }
}
