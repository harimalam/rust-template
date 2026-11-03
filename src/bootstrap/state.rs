use std::sync::Arc;

use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;

use crate::{
    config::Settings,
    infra::{
        cache::Cache,
        db,
        mail::{Mailer, SmtpMailer},
        queue::{AmqpPublisher, JobPublisher},
        storage::{ObjectStorage, S3Storage},
    },
    modules::auth::jwt::JwtKeys,
};

/// Everything a request handler can reach. Cheap to clone (all `Arc`s / pools).
/// External systems are trait objects so tests can substitute fakes.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: PgPool,
    pub cache: Cache,
    pub mailer: Arc<dyn Mailer>,
    pub storage: Arc<dyn ObjectStorage>,
    pub jobs: Arc<dyn JobPublisher>,
    pub jwt: JwtKeys,
    pub metrics: Option<PrometheusHandle>,
}

impl AppState {
    /// Connects every real dependency. Used by the `serve` command.
    pub async fn connect(
        settings: Settings,
        metrics: Option<PrometheusHandle>,
    ) -> anyhow::Result<Self> {
        let db = db::connect(&settings.database).await?;
        if settings.database.run_migrations_on_start {
            db::migrate(&db).await?;
        }
        let cache = Cache::connect(&settings.cache).await?;
        let storage = S3Storage::connect(&settings.s3).await?;
        let jobs = AmqpPublisher::connect(&settings.queue).await?;
        let mailer = SmtpMailer::new(&settings.smtp)?;
        let jwt = JwtKeys::new(&settings.auth);
        Ok(Self {
            settings: Arc::new(settings),
            db,
            cache,
            mailer: Arc::new(mailer),
            storage: Arc::new(storage),
            jobs,
            jwt,
            metrics,
        })
    }
}
