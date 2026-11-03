mod common;

use axum::http::StatusCode;
use common::TestApp;
use sqlx::PgPool;

const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[sqlx::test(migrator = "MIGRATOR")]
async fn liveness_and_readiness(pool: PgPool) {
    let app = TestApp::spawn(pool).await;

    let live = app.get("/health/live", None).await;
    assert_eq!(live.status, StatusCode::OK);
    assert_eq!(live.body["status"], "ok");

    let ready = app.get("/health/ready", None).await;
    assert_eq!(ready.status, StatusCode::OK, "{}", ready.body);
    assert_eq!(ready.body["checks"]["database"], "ok");
    assert_eq!(ready.body["checks"]["cache"], "ok");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn readiness_reports_a_down_database(pool: PgPool) {
    let app = TestApp::spawn(pool.clone()).await;
    pool.close().await;
    let ready = app.get("/health/ready", None).await;
    assert_eq!(ready.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(ready.body["status"], "degraded");
    assert_ne!(ready.body["checks"]["database"], "ok");
    // Liveness is independent of dependencies.
    assert_eq!(app.get("/health/live", None).await.status, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn request_ids_are_generated_and_propagated(pool: PgPool) {
    let app = TestApp::spawn(pool).await;

    let generated = app.get("/health/live", None).await;
    assert!(generated.headers.contains_key("x-request-id"));

    let req = axum::http::Request::builder()
        .uri("/api/v1/nope")
        .header("x-request-id", "trace-abc.123")
        .body(axum::body::Body::empty())
        .unwrap();
    let res = app.send(req).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.headers["x-request-id"], "trace-abc.123");
    assert_eq!(res.body["error"]["request_id"], "trace-abc.123");
    assert_eq!(res.error_code(), "not_found");

    // Hostile request IDs are replaced rather than echoed.
    let req = axum::http::Request::builder()
        .uri("/health/live")
        .header("x-request-id", "bad id with spaces")
        .body(axum::body::Body::empty())
        .unwrap();
    let res = app.send(req).await;
    assert_ne!(res.headers["x-request-id"], "bad id with spaces");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn security_headers_are_set(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let res = app.get("/health/live", None).await;
    assert_eq!(res.headers["x-content-type-options"], "nosniff");
    assert_eq!(res.headers["x-frame-options"], "DENY");
    assert_eq!(res.headers["referrer-policy"], "no-referrer");
    assert!(
        res.headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'")
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn oversized_request_bodies_are_rejected(pool: PgPool) {
    let app = TestApp::spawn_with(pool, |vars| {
        vars.insert("MAX_BODY_BYTES".into(), "256".into());
    })
    .await;
    let big = serde_json::json!({ "email": "a@example.com", "password": "x".repeat(1000), "full_name": "n" });
    let res = app.post("/api/v1/auth/register", None, big).await;
    assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(res.error_code(), "payload_too_large");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn cors_allows_only_configured_origins(pool: PgPool) {
    let app = TestApp::spawn_with(pool, |vars| {
        vars.insert(
            "CORS_ALLOWED_ORIGINS".into(),
            "https://app.example.com".into(),
        );
    })
    .await;
    let preflight = |origin: &'static str| {
        axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/api/v1/notes")
            .header("origin", origin)
            .header("access-control-request-method", "POST")
            .body(axum::body::Body::empty())
            .unwrap()
    };
    let ok = app.send(preflight("https://app.example.com")).await;
    assert_eq!(
        ok.headers["access-control-allow-origin"],
        "https://app.example.com"
    );
    let denied = app.send(preflight("https://evil.example.com")).await;
    assert!(!denied.headers.contains_key("access-control-allow-origin"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn openapi_document_is_served_and_covers_the_api(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let ui = app
        .send(
            axum::http::Request::builder()
                .uri("/docs")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(ui.status, StatusCode::OK);

    let spec =
        serde_json::to_value(<app::bootstrap::openapi::ApiDoc as utoipa::OpenApi>::openapi())
            .unwrap();
    let paths = spec["paths"].as_object().unwrap();
    for expected in [
        "/api/v1/auth/login",
        "/api/v1/auth/refresh",
        "/api/v1/notes",
        "/api/v1/notes/{id}",
        "/api/v1/files",
        "/api/v1/users/{id}/roles",
        "/health/ready",
    ] {
        assert!(
            paths.contains_key(expected),
            "OpenAPI is missing {expected}"
        );
    }
    assert!(spec["components"]["securitySchemes"]["bearer"].is_object());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn docs_can_be_disabled(pool: PgPool) {
    let app = TestApp::spawn_with(pool, |vars| {
        vars.insert("DOCS_ENABLED".into(), "false".into());
    })
    .await;
    let res = app
        .send(
            axum::http::Request::builder()
                .uri("/docs")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}
