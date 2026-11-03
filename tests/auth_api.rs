mod common;

use axum::http::StatusCode;
use common::{TEST_PASSWORD, TestApp};
use serde_json::json;
use sqlx::PgPool;

const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn register_body(email: &str) -> serde_json::Value {
    json!({ "email": email, "password": TEST_PASSWORD, "full_name": "Ada Lovelace" })
}

async fn login(app: &TestApp, email: &str, password: &str) -> common::Response {
    app.post(
        "/api/v1/auth/login",
        None,
        json!({ "email": email, "password": password }),
    )
    .await
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn register_verify_login_and_read_profile(pool: PgPool) {
    let app = TestApp::spawn(pool).await;

    let res = app
        .post(
            "/api/v1/auth/register",
            None,
            register_body("ada@example.com"),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.data()["email"], "ada@example.com");
    assert_eq!(res.data()["email_verified"], false);
    assert_eq!(res.data()["roles"], json!(["user"]));
    assert!(
        res.data().get("password_hash").is_none(),
        "password hash must never be exposed"
    );

    // Verification email was queued and contains a usable token.
    assert_eq!(app.jobs.emails_to("ada@example.com").len(), 1);

    // Cannot log in before verifying.
    let res = login(&app, "ada@example.com", TEST_PASSWORD).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    let token = app.jobs.last_token_for("ada@example.com");
    let res = app
        .post("/api/v1/auth/verify-email", None, json!({ "token": token }))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // The token is single use.
    let res = app
        .post("/api/v1/auth/verify-email", None, json!({ "token": token }))
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    let res = login(&app, "ADA@example.com", TEST_PASSWORD).await; // email is case-insensitive
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.data()["token_type"], "Bearer");
    let access = res.data()["access_token"].as_str().unwrap().to_owned();

    let me = app.get("/api/v1/users/me", Some(&access)).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.data()["email"], "ada@example.com");
    assert_eq!(me.data()["email_verified"], true);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn registration_rejects_duplicates_and_invalid_input(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    assert_eq!(
        app.post(
            "/api/v1/auth/register",
            None,
            register_body("dup@example.com")
        )
        .await
        .status,
        StatusCode::CREATED
    );

    let dup = app
        .post(
            "/api/v1/auth/register",
            None,
            register_body("DUP@example.com"),
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(dup.error_code(), "conflict");

    let bad = app
        .post(
            "/api/v1/auth/register",
            None,
            json!({ "email": "not-an-email", "password": "short", "full_name": "" }),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad.error_code(), "validation_failed");
    let fields: Vec<&str> = bad.body["error"]["details"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["email", "full_name", "password"]);

    let malformed = app
        .send(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/auth/register")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn bad_credentials_are_indistinguishable_and_lock_out(pool: PgPool) {
    let app = TestApp::spawn(pool).await; // lockout after 3 failures
    app.create_user("grace@example.com", false).await;

    let wrong_password = login(&app, "grace@example.com", "not-the-password-123").await;
    let unknown_email = login(&app, "nobody@example.com", "not-the-password-123").await;
    assert_eq!(wrong_password.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_email.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        wrong_password.body["error"]["message"],
        unknown_email.body["error"]["message"]
    );

    login(&app, "grace@example.com", "not-the-password-123").await;
    login(&app, "grace@example.com", "not-the-password-123").await;

    // Now locked: even the correct password is refused until the lockout expires.
    let locked = login(&app, "grace@example.com", TEST_PASSWORD).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(locked.headers.contains_key("retry-after"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn successful_login_resets_failure_counter(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    app.create_user("reset@example.com", false).await;
    login(&app, "reset@example.com", "wrong-password-123").await;
    login(&app, "reset@example.com", "wrong-password-123").await;
    assert_eq!(
        login(&app, "reset@example.com", TEST_PASSWORD).await.status,
        StatusCode::OK
    );
    // Counter was cleared, so two more failures don't lock the account.
    login(&app, "reset@example.com", "wrong-password-123").await;
    login(&app, "reset@example.com", "wrong-password-123").await;
    assert_eq!(
        login(&app, "reset@example.com", TEST_PASSWORD).await.status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn refresh_rotates_and_reuse_revokes_the_whole_family(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    app.create_user("rot@example.com", false).await;

    let first = login(&app, "rot@example.com", TEST_PASSWORD).await;
    let r1 = first.data()["refresh_token"].as_str().unwrap().to_owned();

    let second = app
        .post("/api/v1/auth/refresh", None, json!({ "refresh_token": r1 }))
        .await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    let r2 = second.data()["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(r1, r2);

    // Replaying the old token is treated as theft: it fails AND kills the newer token too.
    let replay = app
        .post("/api/v1/auth/refresh", None, json!({ "refresh_token": r1 }))
        .await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    let after = app
        .post("/api/v1/auth/refresh", None, json!({ "refresh_token": r2 }))
        .await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);

    let garbage = app
        .post(
            "/api/v1/auth/refresh",
            None,
            json!({ "refresh_token": "nope" }),
        )
        .await;
    assert_eq!(garbage.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn logout_revokes_the_session(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    app.create_user("out@example.com", false).await;
    let res = login(&app, "out@example.com", TEST_PASSWORD).await;
    let refresh = res.data()["refresh_token"].as_str().unwrap().to_owned();

    let out = app
        .post(
            "/api/v1/auth/logout",
            None,
            json!({ "refresh_token": refresh }),
        )
        .await;
    assert_eq!(out.status, StatusCode::OK);
    let again = app
        .post(
            "/api/v1/auth/refresh",
            None,
            json!({ "refresh_token": refresh }),
        )
        .await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);

    // Logging out twice (or with an unknown token) is harmless.
    let twice = app
        .post(
            "/api/v1/auth/logout",
            None,
            json!({ "refresh_token": refresh }),
        )
        .await;
    assert_eq!(twice.status, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn password_reset_flow_ends_all_sessions(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    app.create_user("pw@example.com", false).await;
    let session = login(&app, "pw@example.com", TEST_PASSWORD).await;
    let old_refresh = session.data()["refresh_token"].as_str().unwrap().to_owned();

    // Unknown address: same response, nothing sent.
    let unknown = app
        .post(
            "/api/v1/auth/forgot-password",
            None,
            json!({ "email": "ghost@example.com" }),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::OK);
    assert!(app.jobs.emails_to("ghost@example.com").is_empty());

    let res = app
        .post(
            "/api/v1/auth/forgot-password",
            None,
            json!({ "email": "pw@example.com" }),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let token = app.jobs.last_token_for("pw@example.com");

    let weak = app
        .post(
            "/api/v1/auth/reset-password",
            None,
            json!({ "token": token, "new_password": "short" }),
        )
        .await;
    assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);

    let new_password = "a brand new passphrase 42";
    let done = app
        .post(
            "/api/v1/auth/reset-password",
            None,
            json!({ "token": token, "new_password": new_password }),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);

    assert_eq!(
        login(&app, "pw@example.com", TEST_PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(&app, "pw@example.com", new_password).await.status,
        StatusCode::OK
    );
    let stale = app
        .post(
            "/api/v1/auth/refresh",
            None,
            json!({ "refresh_token": old_refresh }),
        )
        .await;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED);

    // Reset tokens are single use.
    let reuse = app
        .post(
            "/api/v1/auth/reset-password",
            None,
            json!({ "token": token, "new_password": new_password }),
        )
        .await;
    assert_eq!(reuse.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn resending_verification_invalidates_the_previous_link(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    app.post(
        "/api/v1/auth/register",
        None,
        register_body("again@example.com"),
    )
    .await;
    let first = app.jobs.last_token_for("again@example.com");

    let res = app
        .post(
            "/api/v1/auth/resend-verification",
            None,
            json!({ "email": "again@example.com" }),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let second = app.jobs.last_token_for("again@example.com");
    assert_ne!(first, second);

    assert_eq!(
        app.post("/api/v1/auth/verify-email", None, json!({ "token": first }))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.post(
            "/api/v1/auth/verify-email",
            None,
            json!({ "token": second })
        )
        .await
        .status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn auth_endpoints_are_rate_limited_per_ip(pool: PgPool) {
    let app = TestApp::spawn_with(pool, |vars| {
        vars.insert("RATE_LIMIT_AUTH_MAX".into(), "3".into());
    })
    .await;
    for _ in 0..3 {
        let res = login(&app, "x@example.com", "whatever-password").await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    }
    let limited = login(&app, "x@example.com", "whatever-password").await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.error_code(), "rate_limited");
    assert!(limited.headers.contains_key("retry-after"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn protected_routes_reject_missing_and_invalid_tokens(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    assert_eq!(
        app.get("/api/v1/users/me", None).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.get("/api/v1/users/me", Some("garbage")).await.status,
        StatusCode::UNAUTHORIZED
    );
    let res = app.get("/api/v1/notes", None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.error_code(), "unauthorized");
}
