mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;
use sqlx::PgPool;

const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[sqlx::test(migrator = "MIGRATOR")]
async fn admin_endpoints_require_permissions(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("plain@example.com", false).await;
    let admin = app.create_user("boss@example.com", true).await;

    let denied = app.get("/api/v1/users", Some(&user.token)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.error_code(), "forbidden");

    let allowed = app.get("/api/v1/users", Some(&admin.token)).await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.body);
    assert_eq!(allowed.body["meta"]["total"], 2);
    assert!(
        allowed
            .data()
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u.get("password_hash").is_none())
    );

    let url = format!("/api/v1/users/{}/roles", user.id);
    let denied = app
        .put(&url, Some(&user.token), json!({ "roles": ["admin"] }))
        .await;
    assert_eq!(
        denied.status,
        StatusCode::FORBIDDEN,
        "users cannot promote themselves"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn user_listing_supports_search_and_sorting(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let admin = app.create_user("zed-admin@example.com", true).await;
    app.create_user("alice@example.com", false).await;
    app.create_user("bob@example.com", false).await;

    let search = app.get("/api/v1/users?q=alice", Some(&admin.token)).await;
    assert_eq!(search.body["meta"]["total"], 1);
    assert_eq!(search.data()[0]["email"], "alice@example.com");
    assert_eq!(search.data()[0]["roles"], json!(["user"]));

    let sorted = app
        .get("/api/v1/users?sort=email", Some(&admin.token))
        .await;
    let emails: Vec<_> = sorted
        .data()
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["email"].as_str().unwrap())
        .collect();
    assert_eq!(
        emails,
        [
            "alice@example.com",
            "bob@example.com",
            "zed-admin@example.com"
        ]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn role_changes_take_effect_on_the_next_token(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let admin = app.create_user("root@example.com", true).await;
    let user = app.create_user("promote@example.com", false).await;
    let url = format!("/api/v1/users/{}/roles", user.id);

    let res = app
        .put(
            &url,
            Some(&admin.token),
            json!({ "roles": ["user", "admin"] }),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.data()["roles"], json!(["admin", "user"]));

    // The old access token still carries the old permissions (they live until it expires)...
    assert_eq!(
        app.get("/api/v1/users", Some(&user.token)).await.status,
        StatusCode::FORBIDDEN
    );
    // ...a fresh login picks up the new role.
    let login = app
        .post(
            "/api/v1/auth/login",
            None,
            json!({ "email": "promote@example.com", "password": common::TEST_PASSWORD }),
        )
        .await;
    let fresh = login.data()["access_token"].as_str().unwrap();
    assert_eq!(
        app.get("/api/v1/users", Some(fresh)).await.status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn role_assignment_is_validated(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let admin = app.create_user("admin@example.com", true).await;
    let user = app.create_user("target@example.com", false).await;
    let url = format!("/api/v1/users/{}/roles", user.id);

    let unknown = app
        .put(&url, Some(&admin.token), json!({ "roles": ["superhero"] }))
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    // The failed assignment rolled back: the user kept their original role.
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT r.name FROM user_roles ur JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = $1",
    )
    .bind(user.id)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    assert_eq!(roles, ["user"]);

    let empty = app
        .put(&url, Some(&admin.token), json!({ "roles": [] }))
        .await;
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);

    let missing = format!("/api/v1/users/{}/roles", uuid::Uuid::now_v7());
    assert_eq!(
        app.put(&missing, Some(&admin.token), json!({ "roles": ["user"] }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    let self_demote = format!("/api/v1/users/{}/roles", admin.id);
    let res = app
        .put(
            &self_demote,
            Some(&admin.token),
            json!({ "roles": ["user"] }),
        )
        .await;
    assert_eq!(
        res.status,
        StatusCode::BAD_REQUEST,
        "admins cannot remove their own admin role"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn profile_update(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("me@example.com", false).await;
    let res = app
        .patch(
            "/api/v1/users/me",
            Some(&user.token),
            json!({ "full_name": "  New Name " }),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.data()["full_name"], "New Name");
    let bad = app
        .patch(
            "/api/v1/users/me",
            Some(&user.token),
            json!({ "full_name": "" }),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
}
