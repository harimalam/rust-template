//! Authentication business rules. Handlers call these functions; they own the rules and talk to
//! the repository, cache and job queue — never to HTTP types.

use chrono::{Duration as ChronoDuration, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use super::{
    dto::{LoginRequest, RegisterRequest, ResetPasswordRequest, TokenPair, UserDto},
    emails, password,
    rbac::ROLE_USER,
    repository::{self, OneTimeTokenKind, UserRecord},
    tokens,
};
use crate::{
    bootstrap::state::AppState,
    http::{AppError, AppResult},
    jobs::Job,
};

/// Request metadata stored alongside refresh tokens for auditing.
#[derive(Debug, Clone, Default)]
pub struct ClientContext {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn chrono_ttl(d: std::time::Duration) -> ChronoDuration {
    ChronoDuration::seconds(i64::try_from(d.as_secs()).unwrap_or(i64::MAX / 2))
}

pub async fn user_dto<'e>(db: impl PgExecutor<'e>, user: &UserRecord) -> AppResult<UserDto> {
    let roles = repository::role_names(db, user.id).await?;
    Ok(UserDto {
        id: user.id,
        email: user.email.clone(),
        full_name: user.full_name.clone(),
        email_verified: user.email_verified_at.is_some(),
        is_active: user.is_active,
        roles,
        created_at: user.created_at,
    })
}

// ── registration & email verification ───────────────────────────────────────

pub async fn register(state: &AppState, req: RegisterRequest) -> AppResult<UserDto> {
    let email = normalize_email(&req.email);
    let hash = password::hash(req.password).await?;
    let verified_at = (!state.settings.auth.require_email_verification).then(Utc::now);

    // User row and default role must succeed or fail together.
    let mut tx = state.db.begin().await?;
    let user = repository::insert_user(
        &mut *tx,
        Uuid::now_v7(),
        &email,
        &hash,
        req.full_name.trim(),
        verified_at,
    )
    .await
    .map_err(|err| match &err {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::Conflict("an account with this email already exists".into())
        }
        _ => AppError::from(err),
    })?;
    repository::assign_role(&mut *tx, user.id, ROLE_USER).await?;
    let dto = user_dto(&mut *tx, &user).await?;
    tx.commit().await?;

    if user.email_verified_at.is_none() {
        // The account exists either way; a queue hiccup must not fail registration.
        // The user can request another email via /auth/resend-verification.
        if let Err(err) = send_verification_email(state, &user).await {
            tracing::error!(error = ?err, user_id = %user.id, "could not enqueue verification email");
        }
    }
    Ok(dto)
}

async fn send_verification_email(state: &AppState, user: &UserRecord) -> AppResult<()> {
    let token = tokens::generate();
    let expires_at = Utc::now() + chrono_ttl(state.settings.auth.email_verification_ttl);
    repository::invalidate_one_time_tokens(&state.db, user.id, OneTimeTokenKind::EmailVerification)
        .await?;
    repository::insert_one_time_token(
        &state.db,
        Uuid::now_v7(),
        user.id,
        OneTimeTokenKind::EmailVerification,
        &tokens::hash(&token),
        expires_at,
    )
    .await?;
    let email = emails::verification(
        &user.email,
        &user.full_name,
        &state.settings.http.public_base_url,
        &token,
    );
    state
        .jobs
        .publish(Job::SendEmail(email))
        .await
        .map_err(AppError::internal)
}

pub async fn verify_email(state: &AppState, token: &str) -> AppResult<()> {
    let user_id = repository::consume_one_time_token(
        &state.db,
        &tokens::hash(token),
        OneTimeTokenKind::EmailVerification,
    )
    .await?
    .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;
    repository::mark_email_verified(&state.db, user_id).await?;
    Ok(())
}

/// Always succeeds from the caller's point of view so it can't be used to probe for accounts.
pub async fn resend_verification(state: &AppState, email: &str) -> AppResult<()> {
    if let Some(user) = repository::find_user_by_email(&state.db, &normalize_email(email)).await?
        && user.is_active
        && user.email_verified_at.is_none()
    {
        send_verification_email(state, &user).await?;
    }
    Ok(())
}

// ── login / refresh / logout ────────────────────────────────────────────────

fn lockout_key(email: &str) -> String {
    format!("login_fail:{email}")
}

pub async fn login(
    state: &AppState,
    req: LoginRequest,
    ctx: &ClientContext,
) -> AppResult<TokenPair> {
    let email = normalize_email(&req.email);
    let limits = &state.settings.rate_limit;
    let key = lockout_key(&email);

    // Temporary lockout after repeated failures for this email (brute-force protection).
    match state.cache.peek_counter(&key).await {
        Ok((failures, ttl)) if failures >= u64::from(limits.login_max_failed_attempts) => {
            return Err(AppError::TooManyRequests {
                retry_after_secs: ttl.max(1),
            });
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(error = ?err, "login lockout check unavailable"),
    }

    let user = repository::find_user_by_email(&state.db, &email).await?;
    let credentials_ok = match user.as_ref().and_then(|u| u.password_hash.clone()) {
        Some(hash) => password::verify(req.password, hash).await?,
        None => {
            password::verify_dummy(req.password).await;
            false
        }
    };

    let user = match (credentials_ok, user) {
        (true, Some(user)) => user,
        _ => {
            if let Err(err) = state.cache.hit_window(&key, limits.login_lockout).await {
                tracing::warn!(error = ?err, "could not record failed login");
            }
            return Err(AppError::Unauthorized("invalid email or password"));
        }
    };

    if let Err(err) = state.cache.delete_strict(&key).await {
        tracing::warn!(error = ?err, "could not clear login failure counter");
    }
    if !user.is_active {
        return Err(AppError::Forbidden("this account is disabled"));
    }
    if state.settings.auth.require_email_verification && user.email_verified_at.is_none() {
        return Err(AppError::Forbidden("email address is not verified"));
    }

    // New login = new token family.
    let mut conn = state.db.acquire().await?;
    issue_token_pair(state, &mut conn, &user, Uuid::now_v7(), Uuid::now_v7(), ctx).await
}

pub async fn issue_token_pair(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    user: &UserRecord,
    family_id: Uuid,
    refresh_id: Uuid,
    ctx: &ClientContext,
) -> AppResult<TokenPair> {
    let roles = repository::role_names(&mut *conn, user.id).await?;
    let perms = repository::permission_names(&mut *conn, user.id).await?;
    let access_token = state.jwt.issue(user.id, roles, perms)?;

    let refresh_token = tokens::generate();
    repository::insert_refresh_token(
        &mut *conn,
        refresh_id,
        user.id,
        family_id,
        &tokens::hash(&refresh_token),
        Utc::now() + chrono_ttl(state.settings.auth.refresh_ttl),
        ctx.user_agent.as_deref(),
        ctx.ip.as_deref(),
    )
    .await?;

    Ok(TokenPair {
        access_token,
        refresh_token,
        token_type: "Bearer",
        expires_in: state.jwt.access_ttl.as_secs(),
    })
}

/// Rotates a refresh token. Presenting a token that was already used or revoked means it may
/// have been stolen, so the whole family (every descendant of that login) is revoked.
pub async fn refresh(
    state: &AppState,
    refresh_token: &str,
    ctx: &ClientContext,
) -> AppResult<TokenPair> {
    let mut tx = state.db.begin().await?;
    let record = repository::lock_refresh_token(&mut *tx, &tokens::hash(refresh_token))
        .await?
        .ok_or(AppError::Unauthorized("invalid refresh token"))?;

    if record.revoked_at.is_some() {
        repository::revoke_token_family(&mut *tx, record.family_id).await?;
        tx.commit().await?; // the revocation must persist even though we return an error
        tracing::warn!(user_id = %record.user_id, family_id = %record.family_id, "refresh token reuse detected; family revoked");
        return Err(AppError::Unauthorized("refresh token has been revoked"));
    }
    if record.expires_at <= Utc::now() {
        return Err(AppError::Unauthorized("refresh token has expired"));
    }

    let user = repository::find_user_by_id(&mut *tx, record.user_id)
        .await?
        .filter(|u| u.is_active)
        .ok_or(AppError::Unauthorized("account is not available"))?;

    let new_id = Uuid::now_v7();
    repository::revoke_refresh_token(&mut *tx, record.id, Some(new_id)).await?;
    let pair = issue_token_pair(state, &mut tx, &user, record.family_id, new_id, ctx).await?;
    tx.commit().await?;
    Ok(pair)
}

/// Idempotent: unknown or already-revoked tokens are not an error.
pub async fn logout(state: &AppState, refresh_token: &str) -> AppResult<()> {
    if let Some(record) =
        repository::lock_refresh_token(&state.db, &tokens::hash(refresh_token)).await?
    {
        repository::revoke_token_family(&state.db, record.family_id).await?;
    }
    Ok(())
}

// ── password reset ──────────────────────────────────────────────────────────

/// Always succeeds from the caller's point of view (no account enumeration).
pub async fn forgot_password(state: &AppState, email: &str) -> AppResult<()> {
    let Some(user) = repository::find_user_by_email(&state.db, &normalize_email(email))
        .await?
        .filter(|u| u.is_active)
    else {
        return Ok(());
    };

    let token = tokens::generate();
    repository::invalidate_one_time_tokens(&state.db, user.id, OneTimeTokenKind::PasswordReset)
        .await?;
    repository::insert_one_time_token(
        &state.db,
        Uuid::now_v7(),
        user.id,
        OneTimeTokenKind::PasswordReset,
        &tokens::hash(&token),
        Utc::now() + chrono_ttl(state.settings.auth.password_reset_ttl),
    )
    .await?;
    let email = emails::password_reset(
        &user.email,
        &user.full_name,
        &state.settings.http.public_base_url,
        &token,
    );
    state
        .jobs
        .publish(Job::SendEmail(email))
        .await
        .map_err(AppError::internal)
}

pub async fn reset_password(state: &AppState, req: ResetPasswordRequest) -> AppResult<()> {
    let hash = password::hash(req.new_password).await?;
    let mut tx = state.db.begin().await?;
    let user_id = repository::consume_one_time_token(
        &mut *tx,
        &tokens::hash(&req.token),
        OneTimeTokenKind::PasswordReset,
    )
    .await?
    .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;

    repository::set_password_hash(&mut *tx, user_id, &hash).await?;
    // Receiving the reset email proves control of the address.
    repository::mark_email_verified(&mut *tx, user_id).await?;
    // A reset invalidates every existing session.
    repository::revoke_all_user_tokens(&mut *tx, user_id).await?;
    let user = repository::find_user_by_id(&mut *tx, user_id).await?;
    tx.commit().await?;

    if let Some(user) = user {
        // Lift any login lockout so the user can sign in with the new password.
        let _ = state.cache.delete_strict(&lockout_key(&user.email)).await;
    }
    Ok(())
}

/// Creates a verified admin account. Used by the `create-admin` CLI command to bootstrap the
/// first administrator (there is intentionally no public endpoint for this).
pub async fn create_admin(
    db: &sqlx::PgPool,
    email: &str,
    password: String,
    full_name: &str,
) -> AppResult<UserDto> {
    let hash = password::hash(password).await?;
    let mut tx = db.begin().await?;
    let user = repository::insert_user(
        &mut *tx,
        Uuid::now_v7(),
        &normalize_email(email),
        &hash,
        full_name,
        Some(Utc::now()),
    )
    .await?;
    repository::assign_role(&mut *tx, user.id, ROLE_USER).await?;
    repository::assign_role(&mut *tx, user.id, super::rbac::ROLE_ADMIN).await?;
    let dto = user_dto(&mut *tx, &user).await?;
    tx.commit().await?;
    Ok(dto)
}
