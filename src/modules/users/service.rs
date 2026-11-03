use std::collections::HashSet;

use uuid::Uuid;

use super::{dto::SetRolesRequest, repository};
use crate::{
    bootstrap::state::AppState,
    http::{
        AppError, AppResult,
        pagination::{ListQuery, Sort},
        response::PageMeta,
    },
    modules::auth::{
        dto::UserDto,
        rbac::{AuthUser, ROLE_ADMIN},
        repository as auth_repo,
        service::user_dto,
    },
};

pub async fn me(state: &AppState, caller: &AuthUser) -> AppResult<UserDto> {
    let user = auth_repo::find_user_by_id(&state.db, caller.id)
        .await?
        .ok_or(AppError::Unauthorized("account no longer exists"))?;
    user_dto(&state.db, &user).await
}

pub async fn update_me(state: &AppState, caller: &AuthUser, full_name: &str) -> AppResult<UserDto> {
    if repository::update_full_name(&state.db, caller.id, full_name.trim()).await? == 0 {
        return Err(AppError::NotFound("user"));
    }
    me(state, caller).await
}

pub async fn list(state: &AppState, query: &ListQuery) -> AppResult<(Vec<UserDto>, PageMeta)> {
    let page = query.page()?;
    let sort = query.sort(
        repository::SORTABLE,
        Sort {
            column: "created_at",
            descending: true,
        },
    )?;
    let search = query.search_pattern();
    let (users, total) = repository::list(&state.db, page, sort, search.as_deref()).await?;

    let ids: Vec<Uuid> = users.iter().map(|u| u.id).collect();
    let mut roles = repository::roles_for_users(&state.db, &ids).await?;
    let dtos = users
        .into_iter()
        .map(|u| UserDto {
            roles: roles.remove(&u.id).unwrap_or_default(),
            id: u.id,
            email: u.email,
            full_name: u.full_name,
            email_verified: u.email_verified_at.is_some(),
            is_active: u.is_active,
            created_at: u.created_at,
        })
        .collect();
    Ok((dtos, PageMeta::new(page.page, page.per_page, total)))
}

pub async fn set_roles(
    state: &AppState,
    caller: &AuthUser,
    target: Uuid,
    req: SetRolesRequest,
) -> AppResult<UserDto> {
    let wanted: HashSet<&String> = req.roles.iter().collect();
    // Prevent an admin from locking themselves out of administration.
    if target == caller.id && !wanted.iter().any(|r| r.as_str() == ROLE_ADMIN) {
        return Err(AppError::BadRequest(
            "you cannot remove your own admin role".into(),
        ));
    }
    let user = auth_repo::find_user_by_id(&state.db, target)
        .await?
        .ok_or(AppError::NotFound("user"))?;

    let unique: Vec<String> = wanted.into_iter().cloned().collect();
    let mut tx = state.db.begin().await?;
    let applied = repository::replace_roles(&mut tx, target, &unique).await?;
    if applied as usize != unique.len() {
        return Err(AppError::BadRequest(
            "one or more roles do not exist".into(),
        ));
    }
    let dto = user_dto(&mut *tx, &user).await?;
    tx.commit().await?;
    Ok(dto)
}
