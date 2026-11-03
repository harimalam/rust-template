use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use futures_util::StreamExt;
use lapin::{
    Channel, Connection, ConnectionProperties,
    message::Delivery,
    options::{BasicAckOptions, BasicCancelOptions, BasicConsumeOptions, BasicQosOptions},
    types::{AMQPValue, FieldTable},
};
use tokio::task::JoinSet;
use tracing::Instrument;

use super::{
    publisher::{ATTEMPTS_HEADER, attempts_header, publish_confirmed},
    topology,
};
use crate::{config::QueueSettings, jobs::JobEnvelope};

#[async_trait]
pub trait JobHandler: Send + Sync {
    async fn handle(&self, envelope: &JobEnvelope) -> anyhow::Result<()>;
}

fn attempts_from(delivery: &Delivery) -> u32 {
    delivery
        .properties
        .headers()
        .as_ref()
        .and_then(|h| h.inner().get(ATTEMPTS_HEADER))
        .and_then(|v| match v {
            AMQPValue::LongUInt(n) => Some(*n),
            AMQPValue::LongInt(n) => u32::try_from(*n).ok(),
            AMQPValue::LongLongInt(n) => u32::try_from(*n).ok(),
            AMQPValue::ShortUInt(n) => Some(u32::from(*n)),
            _ => None,
        })
        .unwrap_or(0)
}

/// Runs the worker until `shutdown` resolves, then stops consuming and waits (up to
/// `drain_timeout`) for in-flight jobs. Returns an error if the broker connection drops, so the
/// orchestrator restarts the process.
pub async fn run_worker<S>(
    settings: &QueueSettings,
    handler: Arc<dyn JobHandler>,
    shutdown: S,
    drain_timeout: Duration,
) -> anyhow::Result<()>
where
    S: Future<Output = ()>,
{
    let connection = Connection::connect(&settings.url, ConnectionProperties::default())
        .await
        .context("failed to connect to RabbitMQ")?;
    let channel = connection.create_channel().await?;
    topology::declare(&channel, settings).await?;
    // Separate confirm-enabled channel for retry / dead-letter publishes.
    let publish_channel = connection.create_channel().await?;
    publish_channel.confirm_select(Default::default()).await?;
    channel
        .basic_qos(settings.prefetch, BasicQosOptions::default())
        .await?;

    let tag = format!("worker-{}", uuid::Uuid::now_v7());
    let mut consumer = channel
        .basic_consume(
            topology::JOBS_QUEUE.into(),
            tag.as_str().into(),
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;
    tracing::info!(
        queue = topology::JOBS_QUEUE,
        prefetch = settings.prefetch,
        "worker started"
    );

    let mut in_flight: JoinSet<()> = JoinSet::new();
    tokio::pin!(shutdown);
    let outcome = loop {
        tokio::select! {
            _ = &mut shutdown => break Ok(()),
            // Reap finished tasks so the set doesn't grow unbounded.
            Some(_) = in_flight.join_next(), if !in_flight.is_empty() => {}
            next = consumer.next() => match next {
                Some(Ok(delivery)) => {
                    in_flight.spawn(process(
                        delivery,
                        handler.clone(),
                        publish_channel.clone(),
                        settings.clone(),
                    ));
                }
                Some(Err(err)) => break Err(anyhow::Error::new(err).context("consumer error")),
                None => break Err(anyhow::anyhow!("RabbitMQ consumer stream ended")),
            },
        }
    };

    tracing::info!("worker stopping; waiting for in-flight jobs");
    let _ = channel
        .basic_cancel(tag.as_str().into(), BasicCancelOptions::default())
        .await;
    let drain = async { while in_flight.join_next().await.is_some() {} };
    if tokio::time::timeout(drain_timeout, drain).await.is_err() {
        tracing::warn!("drain timeout reached; unfinished jobs will be redelivered");
    }
    let _ = connection.close(200, "worker shutdown".into()).await;
    outcome
}

async fn process(
    delivery: Delivery,
    handler: Arc<dyn JobHandler>,
    publish_channel: Channel,
    settings: QueueSettings,
) {
    let attempts = attempts_from(&delivery);

    let envelope: JobEnvelope = match serde_json::from_slice(&delivery.data) {
        Ok(e) => e,
        Err(err) => {
            tracing::error!(%err, "undecodable job message; dead-lettering");
            let outcome =
                dead_letter(&publish_channel, &delivery.data, attempts, &err.to_string()).await;
            finish(&delivery, outcome).await;
            return;
        }
    };

    let span = tracing::info_span!(
        "job",
        job_id = %envelope.id,
        job = envelope.job.name(),
        request_id = envelope.request_id.as_deref().unwrap_or("-"),
        attempt = attempts + 1,
    );
    run_job(
        &delivery,
        &envelope,
        attempts,
        handler,
        &publish_channel,
        &settings,
    )
    .instrument(span)
    .await;
}

async fn run_job(
    delivery: &Delivery,
    envelope: &JobEnvelope,
    attempts: u32,
    handler: Arc<dyn JobHandler>,
    publish_channel: &Channel,
    settings: &QueueSettings,
) {
    match handler.handle(envelope).await {
        Ok(()) => {
            tracing::info!("job completed");
            finish(delivery, Ok(())).await;
        }
        Err(err) => {
            let failed_attempts = attempts + 1;
            if failed_attempts >= settings.job_max_attempts {
                tracing::error!(error = ?err, failed_attempts, "job failed permanently; dead-lettering");
                let outcome = dead_letter(
                    publish_channel,
                    &delivery.data,
                    failed_attempts,
                    &format!("{err:#}"),
                )
                .await;
                finish(delivery, outcome).await;
            } else {
                let delay = topology::retry_delay_secs(settings.retry_base_delay, failed_attempts);
                tracing::warn!(error = ?err, failed_attempts, delay_secs = delay, "job failed; scheduling retry");
                let queue = topology::retry_queue_name(delay);
                let republished = publish_confirmed(
                    publish_channel,
                    topology::RETRY_EXCHANGE,
                    &queue,
                    &delivery.data,
                    attempts_header(failed_attempts),
                )
                .await;
                finish(delivery, republished).await;
            }
        }
    }
}

async fn dead_letter(
    channel: &Channel,
    payload: &[u8],
    attempts: u32,
    reason: &str,
) -> anyhow::Result<()> {
    let mut headers = attempts_header(attempts);
    headers.insert(
        "x-failure-reason".into(),
        AMQPValue::LongString(reason.into()),
    );
    publish_confirmed(
        channel,
        topology::DEAD_EXCHANGE,
        topology::DEAD_ROUTING_KEY,
        payload,
        headers,
    )
    .await
}

/// Acks when `outcome` is Ok. If re-publishing failed, leave the message unacked so the broker
/// redelivers it once this channel closes (no job is lost).
async fn finish(delivery: &Delivery, outcome: anyhow::Result<()>) {
    match outcome {
        Ok(()) => {
            if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
                tracing::error!(%err, "failed to ack message");
            }
        }
        Err(err) => {
            tracing::error!(error = ?err, "could not re-publish failed job; leaving it unacked");
        }
    }
}
