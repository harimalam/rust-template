use axum::{extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    domain::NoteFilter,
    dto::{CreateNoteRequest, NoteDto, NoteFilterQuery, UpdateNoteRequest},
    service,
};
use crate::{
    bootstrap::state::AppState,
    http::{
        AppResult,
        error::ErrorBody,
        extractors::{ApiPath as Path, ApiQuery as Query, ValidatedJson},
        pagination::ListQuery,
        response::{ApiResponse, Envelope, created, ok, paged},
    },
    modules::auth::rbac::{AuthUser, perm},
};

/// Create a note.
#[utoipa::path(
    post, path = "/api/v1/notes", tag = "notes",
    security(("bearer" = [])),
    request_body = CreateNoteRequest,
    responses(
        (status = 201, body = Envelope<NoteDto>),
        (status = 401, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 422, body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ValidatedJson(req): ValidatedJson<CreateNoteRequest>,
) -> AppResult<ApiResponse<NoteDto>> {
    user.require(perm::NOTES_WRITE)?;
    Ok(created(service::create(&state, user.id, req).await?.into()))
}

/// List the caller's notes with pagination, sorting (`created_at`, `updated_at`, `title`),
/// search (`q`, matches title and body) and a `pinned` filter.
#[utoipa::path(
    get, path = "/api/v1/notes", tag = "notes",
    security(("bearer" = [])),
    params(ListQuery, NoteFilterQuery),
    responses(
        (status = 200, body = Envelope<Vec<NoteDto>>),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ListQuery>,
    Query(filter): Query<NoteFilterQuery>,
) -> AppResult<ApiResponse<Vec<NoteDto>>> {
    user.require(perm::NOTES_READ)?;
    let filter = NoteFilter {
        pinned: filter.pinned,
    };
    let (notes, meta) = service::list(&state, user.id, &query, &filter).await?;
    Ok(paged(notes.into_iter().map(Into::into).collect(), meta))
}

/// Get one of the caller's notes (served from cache when possible).
#[utoipa::path(
    get, path = "/api/v1/notes/{id}", tag = "notes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Note ID")),
    responses(
        (status = 200, body = Envelope<NoteDto>),
        (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<ApiResponse<NoteDto>> {
    user.require(perm::NOTES_READ)?;
    Ok(ok(service::get(&state, user.id, id).await?.into()))
}

/// Partially update a note; omitted fields are unchanged.
#[utoipa::path(
    patch, path = "/api/v1/notes/{id}", tag = "notes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Note ID")),
    request_body = UpdateNoteRequest,
    responses(
        (status = 200, body = Envelope<NoteDto>),
        (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 422, body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    ValidatedJson(req): ValidatedJson<UpdateNoteRequest>,
) -> AppResult<ApiResponse<NoteDto>> {
    user.require(perm::NOTES_WRITE)?;
    Ok(ok(service::update(&state, user.id, id, req.into())
        .await?
        .into()))
}

/// Soft-delete a note (permanently purged later by the scheduler).
#[utoipa::path(
    delete, path = "/api/v1/notes/{id}", tag = "notes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Note ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    user.require(perm::NOTES_WRITE)?;
    service::delete(&state, user.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
