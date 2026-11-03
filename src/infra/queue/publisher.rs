use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use lapin::{
    BasicProperties, Channel, Connection, ConnectionProperties,
    options::{BasicPublishOptions, ConfirmSelectOptions},
    types::{AMQPValue, FieldTable},
};
use tokio::sync::Mutex;

use super::topology;
use crate::{
    config::QueueSettings,
    http::request_context::current_request_id,
    jobs::{Job, JobEnvelope},
};

pub const ATTEMPTS_HEADER: &str = "x-attempts";

#[async_trait]
pub trait JobPublisher: Send + Sync {
    async fn publish(&self, job: Job) -> anyhow::Result<()>;
    async fn ping(&self) -> anyhow::Result<()>;
}

/// Publishes one message with persistent delivery and waits for the broker's confirm.
pub(super) async fn publish_confirmed(
    channel: &Channel,
    exchange: &str,
    routing_key: &str,
    payload: &[u8],
    headers: FieldTable,
) -> anyhow::Result<()> {
    let properties = BasicProperties::default()
        .with_delivery_mode(2) // persistent
        .with_content_type("application/json".into())
        .with_headers(headers);
    let confirm = channel
        .basic_publish(
            exchange.into(),
            routing_key.into(),
            BasicPublishOptions::default(),
            payload,
            properties,
        )
        .await?
        .await?;
    anyhow::ensure!(
        confirm.is_ack(),
        "broker did not acknowledge the published message"
    );
    Ok(())
}

pub fn attempts_header(attempts: u32) -> FieldTable {
    let mut headers = FieldTable::default();
    headers.insert(ATTEMPTS_HEADER.into(), AMQPValue::LongUInt(attempts));
    headers
}

struct Inner {
    connection: Connection,
    channel: Channel,
}

/// Publisher with publisher-confirms and transparent reconnect.
pub struct AmqpPublisher {
    settings: QueueSettings,
    inner: Mutex<Inner>,
}

async fn open(settings: &QueueSettings) -> anyhow::Result<Inner> {
    let connection = Connection::connect(&settings.url, ConnectionProperties::default())
        .await
        .context("failed to connect to RabbitMQ")?;
    let channel = connection.create_channel().await?;
    topology::declare(&channel, settings)
        .await
        .context("failed to declare RabbitMQ topology")?;
    channel
        .confirm_select(ConfirmSelectOptions::default())
        .await?;
    Ok(Inner {
        connection,
        channel,
    })
}

impl AmqpPublisher {
    pub async fn connect(settings: &QueueSettings) -> anyhow::Result<Arc<Self>> {
        let inner = open(settings).await?;
        Ok(Arc::new(Self {
            settings: settings.clone(),
            inner: Mutex::new(inner),
        }))
    }

    /// Returns a healthy channel, reconnecting if the broker connection was lost.
    async fn channel(&self) -> anyhow::Result<Channel> {
        let mut inner = self.inner.lock().await;
        if !inner.channel.status().connected() {
            if inner.connection.status().connected() {
                let channel = inner.connection.create_channel().await?;
                channel
                    .confirm_select(ConfirmSelectOptions::default())
                    .await?;
                inner.channel = channel;
            } else {
                tracing::warn!("RabbitMQ connection lost; reconnecting");
                *inner = open(&self.settings).await?;
            }
        }
        Ok(inner.channel.clone())
    }
}

#[async_trait]
impl JobPublisher for AmqpPublisher {
    async fn publish(&self, job: Job) -> anyhow::Result<()> {
        let envelope = JobEnvelope::new(job, current_request_id());
        let payload = serde_json::to_vec(&envelope)?;
        let channel = self.channel().await?;
        publish_confirmed(
            &channel,
            topology::JOBS_EXCHANGE,
            topology::JOBS_ROUTING_KEY,
            &payload,
            attempts_header(0),
        )
        .await
        .with_context(|| format!("failed to publish {} job", envelope.job.name()))?;
        tracing::debug!(job_id = %envelope.id, job = envelope.job.name(), "job enqueued");
        Ok(())
    }

    async fn ping(&self) -> anyhow::Result<()> {
        let channel = self.channel().await?;
        anyhow::ensure!(
            channel.status().connected(),
            "RabbitMQ channel is not connected"
        );
        Ok(())
    }
}
