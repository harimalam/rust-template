//! Shared integration-test harness.
//!
//! * Postgres: each `#[sqlx::test]` gets its own freshly migrated database.
//! * Valkey: the real server, isolated per test by a unique `CACHE_KEY_PREFIX`.
//! * Queue / mail / object storage: in-memory fakes behind the production traits.
#![allow(dead_code)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use app::{
    AppState, build_router,
    config::Settings,
    infra::{
        cache::Cache,
        mail::{Email, Mailer},
        queue::JobPublisher,
        storage::ObjectStorage,
    },
    jobs::Job,
    modules::auth::{jwt::JwtKeys, password, rbac::ROLE_ADMIN, rbac::ROLE_USER, repository},
};
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub const TEST_PASSWORD: &str = "correct horse battery staple";

// ── fakes ───────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct RecordingJobs {
    jobs: Mutex<Vec<Job>>,
}

impl RecordingJobs {
    // `if let` keeps this compiling when more job types are added.
    #[allow(irrefutable_let_patterns)]
    pub fn emails(&self) -> Vec<Email> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|job| {
                if let Job::SendEmail(e) = job {
                    Some(e.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn emails_to(&self, to: &str) -> Vec<Email> {
        self.emails()
            .into_iter()
            .filter(|e| e.to.eq_ignore_ascii_case(to))
            .collect()
    }

    /// The most recent email to `to`, with the token pulled out of its text body.
    pub fn last_token_for(&self, to: &str) -> String {
        let email = self
            .emails_to(to)
            .pop()
            .expect("no email was queued for this address");
        email
            .text
            .split("token=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("email has no token link")
            .to_owned()
    }
}

#[async_trait]
impl JobPublisher for RecordingJobs {
    async fn publish(&self, job: Job) -> anyhow::Result<()> {
        self.jobs.lock().unwrap().push(job);
        Ok(())
    }

    async fn ping(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryStorage {
    objects: Mutex<HashMap<String, (String, Vec<u8>)>>,
}

impl MemoryStorage {
    pub fn len(&self) -> usize {
        self.objects.lock().unwrap().len()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.objects.lock().unwrap().contains_key(key)
    }
}

#[async_trait]
impl ObjectStorage for MemoryStorage {
    async fn put(&self, key: &str, content_type: &str, data: Vec<u8>) -> anyhow::Result<()> {
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_owned(), (content_type.to_owned(), data));
        Ok(())
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }

    async fn presigned_get_url(
        &self,
        key: &str,
        _download_filename: &str,
        ttl: Duration,
    ) -> anyhow::Result<String> {
        Ok(format!(
            "http://storage.test/{key}?expires_in={}",
            ttl.as_secs()
        ))
    }

    async fn ping(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

struct NoopMailer;

#[async_trait]
impl Mailer for NoopMailer {
    async fn send(&self, _email: &Email) -> anyhow::Result<()> {
        Ok(())
    }
}

// ── app ─────────────────────────────────────────────────────────────────────

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub jobs: Arc<RecordingJobs>,
    pub storage: Arc<MemoryStorage>,
}

pub struct TestUser {
    pub id: Uuid,
    pub email: String,
    pub token: String,
}

pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Response {
    /// `data` field of a success envelope.
    pub fn data(&self) -> &Value {
        &self.body["data"]
    }

    pub fn error_code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

fn base_vars() -> HashMap<String, String> {
    let from_env =
        |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.to_owned());
    [
        ("DATABASE_URL", "postgres://unused/unused".to_owned()),
        (
            "VALKEY_URL",
            from_env("VALKEY_URL", "redis://localhost:6380"),
        ),
        (
            "JWT_SECRET",
            "test-secret-test-secret-test-secret-0123".to_owned(),
        ),
        ("SMTP_HOST", "localhost".to_owned()),
        ("SMTP_FROM", "Test <test@example.com>".to_owned()),
        ("S3_ENDPOINT", "http://localhost:9000".to_owned()),
        ("S3_BUCKET", "test".to_owned()),
        ("S3_ACCESS_KEY", "k".to_owned()),
        ("S3_SECRET_KEY", "s".to_owned()),
        ("RABBITMQ_URL", "amqp://localhost".to_owned()),
        ("CACHE_KEY_PREFIX", format!("test-{}", Uuid::now_v7())),
        ("PUBLIC_BASE_URL", "http://app.test".to_owned()),
        // Generous by default so unrelated tests never trip the limiter; tests that exercise
        // limits override these.
        ("RATE_LIMIT_AUTH_MAX", "1000".to_owned()),
        ("LOGIN_MAX_FAILED_ATTEMPTS", "3".to_owned()),
        ("MAX_UPLOAD_BYTES", "2048".to_owned()),
        ("DOCS_ENABLED", "true".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

impl TestApp {
    pub async fn spawn(pool: PgPool) -> Self {
        Self::spawn_with(pool, |_| {}).await
    }

    pub async fn spawn_with(
        pool: PgPool,
        customize: impl FnOnce(&mut HashMap<String, String>),
    ) -> Self {
        let _ = dotenvy::dotenv();
        let mut vars = base_vars();
        customize(&mut vars);
        let settings = Settings::from_map(&vars).expect("test settings are valid");

        let cache = Cache::connect(&settings.cache)
            .await
            .expect("Valkey must be running for integration tests (run `make up`)");
        let jobs = Arc::new(RecordingJobs::default());
        let storage = Arc::new(MemoryStorage::default());
        let state = AppState {
            jwt: JwtKeys::new(&settings.auth),
            settings: Arc::new(settings),
            db: pool,
            cache,
            mailer: Arc::new(NoopMailer),
            storage: storage.clone(),
            jobs: jobs.clone(),
            metrics: None,
        };
        // `oneshot` requests carry no socket address; supply one for the IP-based rate limiter.
        let router = build_router(state.clone())
            .layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 7], 4000))));
        Self {
            router,
            state,
            jobs,
            storage,
        }
    }

    /// Sends a request through the full middleware stack.
    pub async fn send(&self, req: Request<Body>) -> Response {
        let response = self
            .router
            .clone()
            .oneshot(req)
            .await
            .expect("router is infallible");
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.expect("readable body").to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Response {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    pub async fn request(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        json: Option<Value>,
    ) -> Response {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = match json {
            Some(value) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        self.send(builder.body(body).unwrap()).await
    }

    pub async fn get(&self, uri: &str, token: Option<&str>) -> Response {
        self.request(Method::GET, uri, token, None).await
    }

    pub async fn post(&self, uri: &str, token: Option<&str>, json: Value) -> Response {
        self.request(Method::POST, uri, token, Some(json)).await
    }

    pub async fn patch(&self, uri: &str, token: Option<&str>, json: Value) -> Response {
        self.request(Method::PATCH, uri, token, Some(json)).await
    }

    pub async fn put(&self, uri: &str, token: Option<&str>, json: Value) -> Response {
        self.request(Method::PUT, uri, token, Some(json)).await
    }

    pub async fn delete(&self, uri: &str, token: Option<&str>) -> Response {
        self.request(Method::DELETE, uri, token, None).await
    }

    pub async fn upload(
        &self,
        token: &str,
        filename: &str,
        content_type: &str,
        data: &[u8],
    ) -> Response {
        let boundary = "----test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/files")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        self.send(req).await
    }

    // ── factories ───────────────────────────────────────────────────────────

    /// Inserts a verified user directly (fast) and mints an access token for them.
    pub async fn create_user(&self, email: &str, admin: bool) -> TestUser {
        let hash = password::hash(TEST_PASSWORD.to_owned())
            .await
            .expect("hash");
        let user = repository::insert_user(
            &self.state.db,
            Uuid::now_v7(),
            email,
            &hash,
            "Test User",
            Some(chrono::Utc::now()),
        )
        .await
        .expect("insert user");
        repository::assign_role(&self.state.db, user.id, ROLE_USER)
            .await
            .unwrap();
        if admin {
            repository::assign_role(&self.state.db, user.id, ROLE_ADMIN)
                .await
                .unwrap();
        }
        let roles = repository::role_names(&self.state.db, user.id)
            .await
            .unwrap();
        let perms = repository::permission_names(&self.state.db, user.id)
            .await
            .unwrap();
        let token = self
            .state
            .jwt
            .issue(user.id, roles, perms)
            .expect("issue token");
        TestUser {
            id: user.id,
            email: user.email,
            token,
        }
    }

    pub async fn create_note(&self, token: &str, title: &str) -> Value {
        let res = self
            .post(
                "/api/v1/notes",
                Some(token),
                serde_json::json!({ "title": title, "body": "body" }),
            )
            .await;
        assert_eq!(
            res.status,
            StatusCode::CREATED,
            "create note failed: {}",
            res.body
        );
        res.data().clone()
    }
}
