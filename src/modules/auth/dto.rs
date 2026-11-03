use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct RegisterRequest {
    #[validate(email(message = "must be a valid email address"), length(max = 254))]
    pub email: String,
    /// 12–128 characters. Length matters more than composition rules.
    #[validate(length(min = 12, max = 128, message = "must be between 12 and 128 characters"))]
    pub password: String,
    #[validate(length(min = 1, max = 100, message = "must be between 1 and 100 characters"))]
    pub full_name: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(email(message = "must be a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, max = 128))]
    pub password: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct RefreshRequest {
    #[validate(length(min = 1, max = 256))]
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct TokenRequest {
    #[validate(length(min = 1, max = 256))]
    pub token: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct EmailRequest {
    #[validate(email(message = "must be a valid email address"))]
    pub email: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ResetPasswordRequest {
    #[validate(length(min = 1, max = 256))]
    pub token: String,
    #[validate(length(min = 12, max = 128, message = "must be between 12 and 128 characters"))]
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    /// Access token lifetime in seconds.
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub full_name: String,
    pub email_verified: bool,
    pub is_active: bool,
    pub roles: Vec<String>,
    pub created_at: DateTime<Utc>,
}
