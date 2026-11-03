//! SQL for users, roles, refresh tokens and one-time tokens. No business rules live here.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: Uuid,
    pub email: String,
    pub password_hash: Option<String>,
    pub full_name: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy)]
pub enum OneTimeTokenKind {
    EmailVerification,
    PasswordReset,
}

impl OneTimeTokenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::EmailVerification => "email_verification",
            Self::PasswordReset => "password_reset",
        }
    }
}

pub async fn insert_user<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    email: &str,
    password_hash: &str,
    full_name: &str,
    email_verified_at: Option<DateTime<Utc>>,
) -> Result<UserRecord, sqlx::Error> {
    sqlx::query_as!(
        UserRecord,
        r#"INSERT INTO users (id, email, password_hash, full_name, email_verified_at)
           VALUES ($1, $2::text::citext, $3, $4, $5)
           RETURNING id, email::text AS "email!", password_hash, full_name,
                     email_verified_at, is_active, created_at"#,
        id,
        email,
        password_hash,
        full_name,
        email_verified_at,
    )
    .fetch_one(db)
    .await
}

pub async fn assign_role<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    role: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT $1, id FROM roles WHERE name = $2
         ON CONFLICT DO NOTHING",
        user_id,
        role,
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

pub async fn find_user_by_email<'e>(
    db: impl PgExecutor<'e>,
    email: &str,
) -> Result<Option<UserRecord>, sqlx::Error> {
    sqlx::query_as!(
        UserRecord,
        r#"SELECT id, email::text AS "email!", password_hash, full_name,
                  email_verified_at, is_active, created_at
           FROM users WHERE email = $1::text::citext AND deleted_at IS NULL"#,
        email,
    )
    .fetch_optional(db)
    .await
}

pub async fn find_user_by_id<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<UserRecord>, sqlx::Error> {
    sqlx::query_as!(
        UserRecord,
        r#"SELECT id, email::text AS "email!", password_hash, full_name,
                  email_verified_at, is_active, created_at
           FROM users WHERE id = $1 AND deleted_at IS NULL"#,
        id,
    )
    .fetch_optional(db)
    .await
}

pub async fn role_names<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT r.name AS "name!" FROM user_roles ur
           JOIN roles r ON r.id = ur.role_id
           WHERE ur.user_id = $1 ORDER BY r.name"#,
        user_id,
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

pub async fn permission_names<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT DISTINCT p.name AS "name!" FROM user_roles ur
           JOIN role_permissions rp ON rp.role_id = ur.role_id
           JOIN permissions p ON p.id = rp.permission_id
           WHERE ur.user_id = $1 ORDER BY 1"#,
        user_id,
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

pub async fn mark_email_verified<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()), updated_at = now()
         WHERE id = $1",
        user_id,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_password_hash<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1",
        user_id,
        password_hash,
    )
    .execute(db)
    .await?;
    Ok(())
}

// ── refresh tokens ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn insert_refresh_token<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    user_id: Uuid,
    family_id: Uuid,
    token_hash: &[u8],
    expires_at: DateTime<Utc>,
    user_agent: Option<&str>,
    ip_address: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO refresh_tokens (id, user_id, family_id, token_hash, expires_at, user_agent, ip_address)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        id,
        user_id,
        family_id,
        token_hash,
        expires_at,
        user_agent,
        ip_address,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Locks the row so two concurrent refreshes with the same token can't both succeed.
pub async fn lock_refresh_token<'e>(
    db: impl PgExecutor<'e>,
    token_hash: &[u8],
) -> Result<Option<RefreshTokenRecord>, sqlx::Error> {
    sqlx::query_as!(
        RefreshTokenRecord,
        "SELECT id, user_id, family_id, expires_at, revoked_at
         FROM refresh_tokens WHERE token_hash = $1 FOR UPDATE",
        token_hash,
    )
    .fetch_optional(db)
    .await
}

pub async fn revoke_refresh_token<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    replaced_by: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE refresh_tokens SET revoked_at = now(), replaced_by = $2 WHERE id = $1",
        id,
        replaced_by,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn revoke_token_family<'e>(
    db: impl PgExecutor<'e>,
    family_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE refresh_tokens SET revoked_at = now() WHERE family_id = $1 AND revoked_at IS NULL",
        family_id,
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

pub async fn revoke_all_user_tokens<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE refresh_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
        user_id,
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

pub async fn purge_expired_refresh_tokens<'e>(db: impl PgExecutor<'e>) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM refresh_tokens
         WHERE expires_at < now() - interval '1 day'
            OR revoked_at < now() - interval '7 days'",
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

// ── one-time tokens ─────────────────────────────────────────────────────────

/// Invalidates any outstanding tokens of this kind so only the newest link works.
pub async fn invalidate_one_time_tokens<'e>(
    db: impl PgExecutor<'e>,
    user_id: Uuid,
    kind: OneTimeTokenKind,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE one_time_tokens SET used_at = now()
         WHERE user_id = $1 AND kind = $2::text::one_time_token_kind AND used_at IS NULL",
        user_id,
        kind.as_str(),
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn insert_one_time_token<'e>(
    db: impl PgExecutor<'e>,
    id: Uuid,
    user_id: Uuid,
    kind: OneTimeTokenKind,
    token_hash: &[u8],
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO one_time_tokens (id, user_id, kind, token_hash, expires_at)
         VALUES ($1, $2, $3::text::one_time_token_kind, $4, $5)",
        id,
        user_id,
        kind.as_str(),
        token_hash,
        expires_at,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Atomically marks a valid, unused, unexpired token as used and returns its owner.
pub async fn consume_one_time_token<'e>(
    db: impl PgExecutor<'e>,
    token_hash: &[u8],
    kind: OneTimeTokenKind,
) -> Result<Option<Uuid>, sqlx::Error> {
    let row = sqlx::query!(
        "UPDATE one_time_tokens SET used_at = now()
         WHERE token_hash = $1 AND kind = $2::text::one_time_token_kind
           AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
        token_hash,
        kind.as_str(),
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.user_id))
}

pub async fn purge_expired_one_time_tokens<'e>(
    db: impl PgExecutor<'e>,
) -> Result<u64, sqlx::Error> {
    let result =
        sqlx::query!("DELETE FROM one_time_tokens WHERE expires_at < now() - interval '1 day'",)
            .execute(db)
            .await?;
    Ok(result.rows_affected())
}
