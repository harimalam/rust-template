use axum::{
    extract::{Multipart, State, multipart::MultipartError},
    http::StatusCode,
};
use uuid::Uuid;

use super::{
    dto::{DownloadUrlDto, FileDto, FileUpload},
    service,
};
use crate::{
    bootstrap::state::AppState,
    http::{
        AppError, AppResult,
        error::ErrorBody,
        extractors::{ApiPath as Path, ApiQuery as Query},
        pagination::ListQuery,
        response::{ApiResponse, Envelope, created, ok, paged},
    },
    modules::auth::rbac::{AuthUser, perm},
};

fn multipart_error(err: MultipartError) -> AppError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::PayloadTooLarge
    } else {
        AppError::BadRequest(format!("invalid multipart body: {}", err.body_text()))
    }
}

/// Upload a file (multipart field `file`). Type and size are enforced; contents are sniffed.
#[utoipa::path(
    post, path = "/api/v1/files", tag = "files",
    security(("bearer" = [])),
    request_body(content = FileUpload, content_type = "multipart/form-data"),
    responses(
        (status = 201, body = Envelope<FileDto>),
        (status = 413, body = ErrorBody),
        (status = 415, body = ErrorBody),
    )
)]
pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<FileDto>> {
    user.require(perm::FILES_WRITE)?;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("file").to_owned();
        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_owned();
        let data = field.bytes().await.map_err(multipart_error)?.to_vec();
        return Ok(created(
            service::upload(&state, user.id, &filename, &content_type, data).await?,
        ));
    }
    Err(AppError::BadRequest(
        "multipart field \"file\" is required".into(),
    ))
}

/// List the caller's files.
#[utoipa::path(
    get, path = "/api/v1/files", tag = "files",
    security(("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = Envelope<Vec<FileDto>>))
)]
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<ApiResponse<Vec<FileDto>>> {
    user.require(perm::FILES_READ)?;
    let (files, meta) = service::list(&state, user.id, &query).await?;
    Ok(paged(files, meta))
}

/// Get a short-lived presigned URL to download the file directly from object storage.
#[utoipa::path(
    get, path = "/api/v1/files/{id}/download-url", tag = "files",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "File ID")),
    responses(
        (status = 200, body = Envelope<DownloadUrlDto>),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn download_url(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<ApiResponse<DownloadUrlDto>> {
    user.require(perm::FILES_READ)?;
    Ok(ok(service::download_url(&state, user.id, id).await?))
}

/// Delete a file.
#[utoipa::path(
    delete, path = "/api/v1/files/{id}", tag = "files",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "File ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    user.require(perm::FILES_WRITE)?;
    service::delete(&state, user.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
