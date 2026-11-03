//! Argon2id password hashing. Hashing is CPU/memory heavy, so it runs on the blocking pool.

use std::sync::OnceLock;

use anyhow::anyhow;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

use crate::http::AppError;

fn hash_blocking(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("password hashing failed: {e}"))
}

fn verify_blocking(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

pub async fn hash(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || hash_blocking(&password))
        .await
        .map_err(AppError::internal)?
        .map_err(AppError::internal)
}

pub async fn verify(password: String, hash: String) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || verify_blocking(&password, &hash))
        .await
        .map_err(AppError::internal)
}

/// Verifies against a throwaway hash so "unknown email" costs the same as "wrong password",
/// which keeps response timing from revealing which emails are registered.
pub async fn verify_dummy(password: String) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let hash = DUMMY
        .get_or_init(|| hash_blocking("dummy-password-for-timing").unwrap_or_default())
        .clone();
    let _ = verify(password, hash).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_then_verify() {
        let hash = hash("correct horse battery staple".into()).await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(
            verify("correct horse battery staple".into(), hash.clone())
                .await
                .unwrap()
        );
        assert!(!verify("wrong".into(), hash).await.unwrap());
    }

    #[tokio::test]
    async fn garbage_hash_never_verifies() {
        assert!(!verify("x".into(), "not-a-hash".into()).await.unwrap());
    }

    #[tokio::test]
    async fn salts_differ() {
        let a = hash("same".into()).await.unwrap();
        let b = hash("same".into()).await.unwrap();
        assert_ne!(a, b);
    }
}
