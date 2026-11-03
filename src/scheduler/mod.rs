//! Cron jobs. Run as a separate process (`app scheduler`) so API replicas stay free of timers.
//!
//! Every tick takes a short Valkey lock (`SET NX PX`), so running several scheduler replicas
//! for availability still executes each job once per tick.

use std::{future::Future, pin::Pin, sync::Arc, time::Instant};

use anyhow::Context;
use sqlx::PgPool;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    bootstrap::shutdown::shutdown_signal,
    config::{SchedulerSettings, Settings},
    infra::{cache::Cache, db},
    modules::{auth::repository as auth_repo, notes::repository as notes_repo},
};

struct SchedulerContext {
    db: PgPool,
    cache: Cache,
    settings: SchedulerSettings,
}

type TaskFuture = Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>;
type Task = fn(Arc<SchedulerContext>) -> TaskFuture;

pub async fn run(settings: Settings) -> anyhow::Result<()> {
    let ctx = Arc::new(SchedulerContext {
        db: db::connect(&settings.database).await?,
        cache: Cache::connect(&settings.cache).await?,
        settings: settings.scheduler.clone(),
    });

    let scheduler = JobScheduler::new()
        .await
        .context("failed to create scheduler")?;
    register(
        &scheduler,
        "purge_expired_tokens",
        &settings.scheduler.cron_purge_expired_tokens,
        ctx.clone(),
        |ctx| Box::pin(purge_expired_tokens(ctx)),
    )
    .await?;
    register(
        &scheduler,
        "purge_deleted_notes",
        &settings.scheduler.cron_purge_deleted_notes,
        ctx.clone(),
        |ctx| Box::pin(purge_deleted_notes(ctx)),
    )
    .await?;
    // Add new jobs here: write an `async fn(Arc<SchedulerContext>) -> anyhow::Result<String>` and register it.

    scheduler
        .start()
        .await
        .context("failed to start scheduler")?;
    tracing::info!("scheduler started");

    shutdown_signal().await;
    let mut scheduler = scheduler;
    scheduler
        .shutdown()
        .await
        .context("failed to stop scheduler")?;
    ctx.db.close().await;
    tracing::info!("scheduler stopped");
    Ok(())
}

async fn register(
    scheduler: &JobScheduler,
    name: &'static str,
    cron: &str,
    ctx: Arc<SchedulerContext>,
    task: Task,
) -> anyhow::Result<()> {
    let job = Job::new_async(cron, move |_id, _lock| {
        let ctx = ctx.clone();
        Box::pin(async move { run_once(name, ctx, task).await })
    })
    .with_context(|| format!("invalid cron expression for {name}: {cron:?}"))?;
    scheduler.add(job).await?;
    tracing::info!(job = name, cron, "scheduled");
    Ok(())
}

async fn run_once(name: &'static str, ctx: Arc<SchedulerContext>, task: Task) {
    // The lock is deliberately not released: it expires after CRON_LOCK_TTL_SECS, which stops a
    // second replica that ticks a few milliseconds later from running the same job again.
    match ctx
        .cache
        .try_lock(&format!("cron:{name}"), ctx.settings.lock_ttl)
        .await
    {
        Ok(Some(_guard)) => {}
        Ok(None) => {
            tracing::debug!(job = name, "skipped; another instance holds the lock");
            return;
        }
        Err(err) => {
            tracing::warn!(job = name, error = ?err, "skipped; could not coordinate via Valkey");
            return;
        }
    }

    let started = Instant::now();
    match task(ctx).await {
        Ok(summary) => {
            metrics::counter!("cron_job_runs_total", "job" => name, "result" => "ok").increment(1);
            tracing::info!(job = name, elapsed_ms = started.elapsed().as_millis() as u64, %summary, "cron job finished");
        }
        Err(err) => {
            metrics::counter!("cron_job_runs_total", "job" => name, "result" => "error")
                .increment(1);
            tracing::error!(job = name, error = ?err, "cron job failed");
        }
    }
}

async fn purge_expired_tokens(ctx: Arc<SchedulerContext>) -> anyhow::Result<String> {
    let refresh = auth_repo::purge_expired_refresh_tokens(&ctx.db).await?;
    let one_time = auth_repo::purge_expired_one_time_tokens(&ctx.db).await?;
    Ok(format!(
        "deleted {refresh} refresh token(s), {one_time} one-time token(s)"
    ))
}

async fn purge_deleted_notes(ctx: Arc<SchedulerContext>) -> anyhow::Result<String> {
    let days = i32::try_from(ctx.settings.soft_delete_retention_days)
        .context("SOFT_DELETE_RETENTION_DAYS out of range")?;
    let deleted = notes_repo::purge_deleted(&ctx.db, days).await?;
    Ok(format!(
        "permanently deleted {deleted} note(s) older than {days} day(s)"
    ))
}
