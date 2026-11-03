use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateProfileRequest {
    #[validate(length(min = 1, max = 100, message = "must be between 1 and 100 characters"))]
    pub full_name: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct SetRolesRequest {
    /// Complete set of roles the user should have, e.g. `["user", "admin"]`.
    #[validate(length(min = 1, max = 10, message = "provide between 1 and 10 roles"))]
    pub roles: Vec<String>,
}
