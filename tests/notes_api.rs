mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::json;
use sqlx::PgPool;

const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[sqlx::test(migrator = "MIGRATOR")]
async fn full_crud_lifecycle(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("owner@example.com", false).await;
    let token = Some(user.token.as_str());

    let created = app
        .post(
            "/api/v1/notes",
            token,
            json!({ "title": "  First  ", "body": "hello", "pinned": true }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.data()["title"], "First", "title is trimmed");
    let id = created.data()["id"].as_str().unwrap().to_owned();
    let url = format!("/api/v1/notes/{id}");

    let fetched = app.get(&url, token).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.data()["body"], "hello");

    let patched = app.patch(&url, token, json!({ "body": "changed" })).await;
    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(patched.data()["body"], "changed");
    assert_eq!(
        patched.data()["title"],
        "First",
        "omitted fields are untouched"
    );
    assert_eq!(patched.data()["pinned"], true);

    let deleted = app.delete(&url, token).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(app.get(&url, token).await.status, StatusCode::NOT_FOUND);
    assert_eq!(app.delete(&url, token).await.status, StatusCode::NOT_FOUND);

    // Soft delete: the row remains for the retention window.
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE deleted_at IS NOT NULL")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(remaining, 1);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn audit_fields_are_maintained(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("audit@example.com", false).await;
    let note = app.create_note(&user.token, "audited").await;
    let id = note["id"].as_str().unwrap();

    app.patch(
        &format!("/api/v1/notes/{id}"),
        Some(&user.token),
        json!({ "title": "renamed" }),
    )
    .await;
    let row: (uuid::Uuid, uuid::Uuid, bool) = sqlx::query_as(
        "SELECT created_by, updated_by, updated_at > created_at FROM notes WHERE id = $1::text::uuid",
    )
    .bind(id)
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(row.0, user.id);
    assert_eq!(row.1, user.id);
    assert!(row.2, "updated_at must advance on update");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn list_supports_pagination_sorting_search_and_filters(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("lister@example.com", false).await;
    let token = Some(user.token.as_str());
    for title in ["delta", "alpha", "charlie", "bravo", "echo"] {
        app.create_note(&user.token, title).await;
    }
    let pinned = app
        .post(
            "/api/v1/notes",
            token,
            json!({ "title": "pinned one", "pinned": true }),
        )
        .await;
    assert_eq!(pinned.status, StatusCode::CREATED);

    let page1 = app
        .get("/api/v1/notes?sort=title&per_page=2&page=1", token)
        .await;
    assert_eq!(page1.status, StatusCode::OK);
    let titles: Vec<_> = page1
        .data()
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["alpha", "bravo"]);
    assert_eq!(page1.body["meta"]["total"], 6);
    assert_eq!(page1.body["meta"]["total_pages"], 3);

    let page3 = app
        .get("/api/v1/notes?sort=title&per_page=2&page=3", token)
        .await;
    let titles: Vec<_> = page3
        .data()
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["echo", "pinned one"]);

    let desc = app.get("/api/v1/notes?sort=-title&per_page=1", token).await;
    assert_eq!(desc.data()[0]["title"], "pinned one");

    let search = app.get("/api/v1/notes?q=ALPH", token).await;
    assert_eq!(search.body["meta"]["total"], 1);
    assert_eq!(search.data()[0]["title"], "alpha");

    let only_pinned = app.get("/api/v1/notes?pinned=true", token).await;
    assert_eq!(only_pinned.body["meta"]["total"], 1);

    // Search treats % and _ literally.
    assert_eq!(
        app.get("/api/v1/notes?q=%25", token).await.body["meta"]["total"],
        0
    );

    assert_eq!(
        app.get("/api/v1/notes?sort=password_hash", token)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get("/api/v1/notes?per_page=1000", token).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get("/api/v1/notes?page=0", token).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn users_cannot_touch_each_others_notes_even_when_cached(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let alice = app.create_user("alice@example.com", false).await;
    let bob = app.create_user("bob@example.com", false).await;
    let note = app.create_note(&alice.token, "alice secret").await;
    let url = format!("/api/v1/notes/{}", note["id"].as_str().unwrap());

    // Alice reads it, which puts it in the cache.
    assert_eq!(
        app.get(&url, Some(&alice.token)).await.status,
        StatusCode::OK
    );

    assert_eq!(
        app.get(&url, Some(&bob.token)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.patch(&url, Some(&bob.token), json!({ "title": "hacked" }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&url, Some(&bob.token)).await.status,
        StatusCode::NOT_FOUND
    );

    let list = app.get("/api/v1/notes", Some(&bob.token)).await;
    assert_eq!(list.body["meta"]["total"], 0);

    let still = app.get(&url, Some(&alice.token)).await;
    assert_eq!(still.data()["title"], "alice secret");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn reads_are_cached_and_writes_invalidate(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("cache@example.com", false).await;
    let token = Some(user.token.as_str());
    let note = app.create_note(&user.token, "cached").await;
    let id = note["id"].as_str().unwrap();
    let url = format!("/api/v1/notes/{id}");

    app.get(&url, token).await; // populates cache

    // Change the row behind the API's back: a cached read must still return the old value...
    sqlx::query("UPDATE notes SET title = 'changed-in-db' WHERE id = $1::text::uuid")
        .bind(id)
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(
        app.get(&url, token).await.data()["title"],
        "cached",
        "second read is served from cache"
    );

    // ...until a write through the API invalidates the entry.
    app.patch(&url, token, json!({ "pinned": true })).await;
    assert_eq!(app.get(&url, token).await.data()["title"], "changed-in-db");

    // Deleting invalidates too, so a cached copy can't resurrect a deleted note.
    app.delete(&url, token).await;
    assert_eq!(app.get(&url, token).await.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn validation_errors_use_the_standard_shape(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("valid@example.com", false).await;
    let res = app
        .post("/api/v1/notes", Some(&user.token), json!({ "title": "" }))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.error_code(), "validation_failed");
    assert_eq!(res.body["error"]["details"][0]["field"], "title");
    assert!(
        res.body["error"]["request_id"].is_string(),
        "errors carry the request id"
    );

    let long = "x".repeat(201);
    let res = app
        .post("/api/v1/notes", Some(&user.token), json!({ "title": long }))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(
        app.get("/api/v1/notes/not-a-uuid", Some(&user.token))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
}
