use utoipa::{
    Modify, OpenApi,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

use crate::modules::{auth, files, health, notes, users};

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Rust Backend Template API",
        description = "Success responses are wrapped in `{ \"data\": ..., \"meta\": ... }`; errors use \
                       `{ \"error\": { \"code\", \"message\", \"details\", \"request_id\" } }`.",
        version = "1.0.0"
    ),
    paths(
        health::live, health::ready,
        auth::handlers::register, auth::handlers::login, auth::handlers::refresh,
        auth::handlers::logout, auth::handlers::verify_email, auth::handlers::resend_verification,
        auth::handlers::forgot_password, auth::handlers::reset_password,
        users::handlers::me, users::handlers::update_me, users::handlers::list,
        users::handlers::set_roles,
        notes::handlers::create, notes::handlers::list, notes::handlers::get,
        notes::handlers::update, notes::handlers::delete,
        files::handlers::upload, files::handlers::list, files::handlers::download_url,
        files::handlers::delete,
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "auth", description = "Registration, login, token refresh, email verification, password reset"),
        (name = "users", description = "Profiles and role management (RBAC)"),
        (name = "notes", description = "Example CRUD resource"),
        (name = "files", description = "Uploads to S3-compatible storage"),
        (name = "health", description = "Liveness and readiness probes"),
    )
)]
pub struct ApiDoc;
