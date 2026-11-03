use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FileDto {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DownloadUrlDto {
    /// Time-limited URL; fetch it directly with GET (no auth header needed).
    pub url: String,
    pub expires_in: u64,
}

/// Schema for the multipart upload body (documentation only).
#[derive(ToSchema)]
pub struct FileUpload {
    #[schema(value_type = String, format = Binary)]
    #[allow(dead_code)]
    pub file: Vec<u8>,
}
