use axum::extract::State;
use uuid::Uuid;

use super::{
    dto::{SetRolesRequest, UpdateProfileRequest},
    service,
};
use crate::{
    bootstrap::state::AppState,
    http::{
        AppResult,
        error::ErrorBody,
        extractors::{ApiPath as Path, ApiQuery as Query, ValidatedJson},
        pagination::ListQuery,
        response::{ApiResponse, Envelope, ok, paged},
    },
    modules::auth::{
        dto::UserDto,
        rbac::{AuthUser, perm},
    },
};

/// The authenticated user's profile.
#[utoipa::path(
    get, path = "/api/v1/users/me", tag = "users",
    security(("bearer" = [])),
    responses(
        (status = 200, body = Envelope<UserDto>),
        (status = 401, body = ErrorBody),
    )
)]
pub async fn me(State(state): State<AppState>, user: AuthUser) -> AppResult<ApiResponse<UserDto>> {
    Ok(ok(service::me(&state, &user).await?))
}

/// Update the authenticated user's profile.
#[utoipa::path(
    patch, path = "/api/v1/users/me", tag = "users",
    security(("bearer" = [])),
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, body = Envelope<UserDto>),
        (status = 401, body = ErrorBody),
        (status = 422, body = ErrorBody),
    )
)]
pub async fn update_me(
    State(state): State<AppState>,
    user: AuthUser,
    ValidatedJson(req): ValidatedJson<UpdateProfileRequest>,
) -> AppResult<ApiResponse<UserDto>> {
    Ok(ok(service::update_me(&state, &user, &req.full_name).await?))
}

/// List users (requires `users:read`). Search matches email and name.
#[utoipa::path(
    get, path = "/api/v1/users", tag = "users",
    security(("bearer" = [])),
    params(ListQuery),
    responses(
        (status = 200, body = Envelope<Vec<UserDto>>),
        (status = 403, body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<ApiResponse<Vec<UserDto>>> {
    user.require(perm::USERS_READ)?;
    let (users, meta) = service::list(&state, &query).await?;
    Ok(paged(users, meta))
}

/// Replace a user's roles (requires `users:manage`).
#[utoipa::path(
    put, path = "/api/v1/users/{id}/roles", tag = "users",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User ID")),
    request_body = SetRolesRequest,
    responses(
        (status = 200, body = Envelope<UserDto>),
        (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
pub async fn set_roles(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    ValidatedJson(req): ValidatedJson<SetRolesRequest>,
) -> AppResult<ApiResponse<UserDto>> {
    user.require(perm::USERS_MANAGE)?;
    Ok(ok(service::set_roles(&state, &user, id, req).await?))
}
