mod common;

use axum::http::StatusCode;
use common::TestApp;
use sqlx::PgPool;

const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];

#[sqlx::test(migrator = "MIGRATOR")]
async fn upload_list_download_and_delete(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let user = app.create_user("files@example.com", false).await;

    let up = app
        .upload(&user.token, "../../evil/name.png", "image/png", PNG)
        .await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.body);
    assert_eq!(
        up.data()["filename"],
        "name.png",
        "path components are stripped"
    );
    assert_eq!(up.data()["size_bytes"], PNG.len());
    let id = up.data()["id"].as_str().unwrap().to_owned();

    // Stored under a server-generated key, not the client's filename.
    let key = format!("{}/{}", user.id, id);
    assert!(app.storage.contains(&key));

    let list = app.get("/api/v1/files", Some(&user.token)).await;
    assert_eq!(list.body["meta"]["total"], 1);

    let url = app
        .get(
            &format!("/api/v1/files/{id}/download-url"),
            Some(&user.token),
        )
        .await;
    assert_eq!(url.status, StatusCode::OK);
    assert!(url.data()["url"].as_str().unwrap().contains(&key));

    let del = app
        .delete(&format!("/api/v1/files/{id}"), Some(&user.token))
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    assert!(!app.storage.contains(&key), "object removed from storage");
    assert_eq!(
        app.get(
            &format!("/api/v1/files/{id}/download-url"),
            Some(&user.token)
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn upload_validation(pool: PgPool) {
    let app = TestApp::spawn(pool).await; // MAX_UPLOAD_BYTES=2048 in tests
    let user = app.create_user("strict@example.com", false).await;

    // Declared type not on the allow-list.
    let exe = app
        .upload(
            &user.token,
            "run.exe",
            "application/x-msdownload",
            b"MZ\x90\x00",
        )
        .await;
    assert_eq!(exe.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Allowed type declared, but the bytes aren't that type.
    let fake = app
        .upload(
            &user.token,
            "fake.png",
            "image/png",
            b"<?php system($_GET['c']); ?>",
        )
        .await;
    assert_eq!(fake.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(
        fake.body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("do not match")
    );

    // Over the size limit.
    let mut big = PNG.to_vec();
    big.resize(4096, 0);
    let too_big = app.upload(&user.token, "big.png", "image/png", &big).await;
    assert_eq!(too_big.status, StatusCode::PAYLOAD_TOO_LARGE);

    let empty = app
        .upload(&user.token, "empty.txt", "text/plain", b"")
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);

    assert_eq!(app.storage.len(), 0, "rejected uploads never reach storage");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn files_are_private_to_their_owner(pool: PgPool) {
    let app = TestApp::spawn(pool).await;
    let alice = app.create_user("alice-f@example.com", false).await;
    let bob = app.create_user("bob-f@example.com", false).await;
    let up = app
        .upload(&alice.token, "a.txt", "text/plain", b"private notes")
        .await;
    assert_eq!(up.status, StatusCode::CREATED, "{}", up.body);
    let id = up.data()["id"].as_str().unwrap();

    let url = format!("/api/v1/files/{id}/download-url");
    assert_eq!(
        app.get(&url, Some(&bob.token)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&format!("/api/v1/files/{id}"), Some(&bob.token))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/api/v1/files", Some(&bob.token)).await.body["meta"]["total"],
        0
    );
    assert_eq!(app.get(&url, None).await.status, StatusCode::UNAUTHORIZED);
}
