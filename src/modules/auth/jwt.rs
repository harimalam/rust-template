//! Short-lived signed access tokens (HS256). Roles and permissions are embedded so requests are
//! authorised without a database round trip; changes take effect when the token expires
//! (`JWT_ACCESS_TTL_SECS`) or on next refresh.

use std::time::Duration;

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{config::AuthSettings, http::AppError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub iss: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: Uuid,
    pub roles: Vec<String>,
    pub perms: Vec<String>,
}

#[derive(Clone)]
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    issuer: String,
    pub access_ttl: Duration,
}

impl JwtKeys {
    pub fn new(settings: &AuthSettings) -> Self {
        Self {
            encoding: EncodingKey::from_secret(settings.jwt_secret.as_bytes()),
            decoding: DecodingKey::from_secret(settings.jwt_secret.as_bytes()),
            issuer: settings.jwt_issuer.clone(),
            access_ttl: settings.access_ttl,
        }
    }

    pub fn issue(
        &self,
        user_id: Uuid,
        roles: Vec<String>,
        perms: Vec<String>,
    ) -> Result<String, AppError> {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            sub: user_id,
            iss: self.issuer.clone(),
            iat: now,
            exp: now + self.access_ttl.as_secs() as i64,
            jti: Uuid::now_v7(),
            roles,
            perms,
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding).map_err(AppError::internal)
    }

    pub fn verify(&self, token: &str) -> Result<Claims, AppError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);
        decode::<Claims>(token, &self.decoding, &validation)
            .map(|data| data.claims)
            .map_err(|_| AppError::Unauthorized("invalid or expired access token"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(secret: &str, ttl_secs: u64) -> JwtKeys {
        JwtKeys::new(&AuthSettings {
            jwt_secret: secret.into(),
            jwt_issuer: "test".into(),
            access_ttl: Duration::from_secs(ttl_secs),
            refresh_ttl: Duration::from_secs(60),
            email_verification_ttl: Duration::from_secs(60),
            password_reset_ttl: Duration::from_secs(60),
            require_email_verification: true,
        })
    }

    #[test]
    fn issue_and_verify_round_trip() {
        let k = keys("0123456789abcdef0123456789abcdef", 60);
        let id = Uuid::now_v7();
        let token = k
            .issue(id, vec!["user".into()], vec!["notes:read".into()])
            .unwrap();
        let claims = k.verify(&token).unwrap();
        assert_eq!(claims.sub, id);
        assert_eq!(claims.perms, vec!["notes:read"]);
    }

    #[test]
    fn rejects_wrong_secret_and_garbage() {
        let a = keys("0123456789abcdef0123456789abcdef", 60);
        let b = keys("fedcba9876543210fedcba9876543210", 60);
        let token = a.issue(Uuid::now_v7(), vec![], vec![]).unwrap();
        assert!(b.verify(&token).is_err());
        assert!(a.verify("not.a.jwt").is_err());
    }

    #[test]
    fn rejects_expired_tokens() {
        let k = keys("0123456789abcdef0123456789abcdef", 0);
        let token = k.issue(Uuid::now_v7(), vec![], vec![]).unwrap();
        // jsonwebtoken allows 60s leeway by default; craft a token that is well past it.
        let claims = Claims {
            sub: Uuid::now_v7(),
            iss: "test".into(),
            iat: 0,
            exp: 1,
            jti: Uuid::now_v7(),
            roles: vec![],
            perms: vec![],
        };
        let old = encode(&Header::new(Algorithm::HS256), &claims, &k.encoding).unwrap();
        assert!(k.verify(&old).is_err());
        let _ = token;
    }
}
