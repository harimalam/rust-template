//! Standard success envelope: `{ "data": ..., "meta": { ... } }`.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PageMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: i64,
    pub total_pages: u32,
}

impl PageMeta {
    pub fn new(page: u32, per_page: u32, total: i64) -> Self {
        let total_pages = if total <= 0 {
            0
        } else {
            ((total as u64).div_ceil(per_page as u64)) as u32
        };
        Self {
            page,
            per_page,
            total,
            total_pages,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Envelope<T> {
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<PageMeta>,
}

/// A successful response. Build with [`ok`], [`created`] or [`paged`].
pub struct ApiResponse<T> {
    status: StatusCode,
    body: Envelope<T>,
}

pub fn ok<T: Serialize>(data: T) -> ApiResponse<T> {
    ApiResponse {
        status: StatusCode::OK,
        body: Envelope { data, meta: None },
    }
}

pub fn created<T: Serialize>(data: T) -> ApiResponse<T> {
    ApiResponse {
        status: StatusCode::CREATED,
        body: Envelope { data, meta: None },
    }
}

pub fn paged<T: Serialize>(data: Vec<T>, meta: PageMeta) -> ApiResponse<Vec<T>> {
    ApiResponse {
        status: StatusCode::OK,
        body: Envelope {
            data,
            meta: Some(meta),
        },
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

/// Plain acknowledgement payload for operations with nothing to return.
#[derive(Debug, Serialize, ToSchema)]
pub struct Message {
    pub message: String,
}

pub fn message(text: impl Into<String>) -> ApiResponse<Message> {
    ok(Message {
        message: text.into(),
    })
}
