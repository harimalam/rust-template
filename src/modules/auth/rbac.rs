//! Authentication extractor and permission checks.

use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts},
};
use uuid::Uuid;

use crate::{bootstrap::state::AppState, http::AppError};

/// Permission names. They match rows in the `permissions` table (see migrations).
pub mod perm {
    pub const NOTES_READ: &str = "notes:read";
    pub const NOTES_WRITE: &str = "notes:write";
    pub const FILES_READ: &str = "files:read";
    pub const FILES_WRITE: &str = "files:write";
    pub const USERS_READ: &str = "users:read";
    pub const USERS_MANAGE: &str = "users:manage";
}

pub const ROLE_USER: &str = "user";
pub const ROLE_ADMIN: &str = "admin";

/// The authenticated caller, built from a verified access token.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
}

impl AuthUser {
    /// Succeeds only if the caller holds `permission`; otherwise 403.
    pub fn require(&self, permission: &str) -> Result<(), AppError> {
        if self.permissions.iter().any(|p| p == permission) {
            Ok(())
        } else {
            Err(AppError::Forbidden(
                "you do not have permission to perform this action",
            ))
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AppError::Unauthorized("missing Authorization header"))?;
        let token = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))
            .ok_or(AppError::Unauthorized(
                "Authorization header must be a Bearer token",
            ))?;
        let claims = state.jwt.verify(token.trim())?;
        Ok(Self {
            id: claims.sub,
            roles: claims.roles,
            permissions: claims.perms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_checks_permission_membership() {
        let user = AuthUser {
            id: Uuid::now_v7(),
            roles: vec![ROLE_USER.into()],
            permissions: vec![perm::NOTES_READ.into()],
        };
        assert!(user.require(perm::NOTES_READ).is_ok());
        assert!(matches!(
            user.require(perm::USERS_MANAGE),
            Err(AppError::Forbidden(_))
        ));
    }
}
