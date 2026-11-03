//! RabbitMQ topology.
//!
//! ```text
//! publish ──► [jobs] ──► jobs.default ──► worker
//!                           ▲                │ failure (attempt < max)
//!   dead-letter after TTL   │                ▼
//!      jobs.retry.<N>s ◄── [jobs.retry] ◄────┤
//!                                            │ failure (attempt >= max / undecodable)
//!                                            ▼
//!                                  [jobs.dead] ──► jobs.dead   (inspect / re-drive manually)
//! ```
//!
//! Each retry delay has its own queue with a fixed `x-message-ttl`. Per-message TTLs on one queue
//! would block shorter delays behind longer ones (RabbitMQ only expires from the queue head).
//! The delay is part of the queue name, so changing `JOB_RETRY_BASE_DELAY_SECS` creates new
//! queues instead of failing with an argument-mismatch error.

use std::time::Duration;

use lapin::{
    Channel, ExchangeKind,
    options::{ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions},
    types::{AMQPValue, FieldTable},
};

use crate::config::QueueSettings;

pub const JOBS_EXCHANGE: &str = "jobs";
pub const JOBS_QUEUE: &str = "jobs.default";
pub const JOBS_ROUTING_KEY: &str = "default";
pub const RETRY_EXCHANGE: &str = "jobs.retry";
pub const DEAD_EXCHANGE: &str = "jobs.dead";
pub const DEAD_QUEUE: &str = "jobs.dead";
pub const DEAD_ROUTING_KEY: &str = "dead";
const MAX_RETRY_DELAY_SECS: u64 = 3600;

/// Delay before re-delivering after the `attempt`-th failure (1-based): base * 2^(attempt-1).
pub fn retry_delay_secs(base: Duration, attempt: u32) -> u64 {
    let factor = 1u64 << attempt.saturating_sub(1).min(20);
    base.as_secs()
        .max(1)
        .saturating_mul(factor)
        .min(MAX_RETRY_DELAY_SECS)
}

pub fn retry_queue_name(delay_secs: u64) -> String {
    format!("jobs.retry.{delay_secs}s")
}

pub async fn declare(channel: &Channel, settings: &QueueSettings) -> anyhow::Result<()> {
    let exchange_opts = ExchangeDeclareOptions {
        durable: true,
        ..Default::default()
    };
    for name in [JOBS_EXCHANGE, RETRY_EXCHANGE, DEAD_EXCHANGE] {
        channel
            .exchange_declare(
                name.into(),
                ExchangeKind::Direct,
                exchange_opts,
                FieldTable::default(),
            )
            .await?;
    }

    channel
        .queue_declare(
            JOBS_QUEUE.into(),
            QueueDeclareOptions::durable(),
            FieldTable::default(),
        )
        .await?;
    channel
        .queue_bind(
            JOBS_QUEUE.into(),
            JOBS_EXCHANGE.into(),
            JOBS_ROUTING_KEY.into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;

    channel
        .queue_declare(
            DEAD_QUEUE.into(),
            QueueDeclareOptions::durable(),
            FieldTable::default(),
        )
        .await?;
    channel
        .queue_bind(
            DEAD_QUEUE.into(),
            DEAD_EXCHANGE.into(),
            DEAD_ROUTING_KEY.into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;

    // One retry queue per distinct delay (failure N retries after delay(N)).
    let mut delays: Vec<u64> = (1..settings.job_max_attempts)
        .map(|attempt| retry_delay_secs(settings.retry_base_delay, attempt))
        .collect();
    delays.dedup();
    for delay in delays {
        let name = retry_queue_name(delay);
        let mut args = FieldTable::default();
        args.insert(
            "x-message-ttl".into(),
            AMQPValue::LongLongInt((delay * 1000) as i64),
        );
        args.insert(
            "x-dead-letter-exchange".into(),
            AMQPValue::LongString(JOBS_EXCHANGE.into()),
        );
        args.insert(
            "x-dead-letter-routing-key".into(),
            AMQPValue::LongString(JOBS_ROUTING_KEY.into()),
        );
        channel
            .queue_declare(name.as_str().into(), QueueDeclareOptions::durable(), args)
            .await?;
        channel
            .queue_bind(
                name.as_str().into(),
                RETRY_EXCHANGE.into(),
                name.as_str().into(),
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_is_capped() {
        let base = Duration::from_secs(5);
        assert_eq!(retry_delay_secs(base, 1), 5);
        assert_eq!(retry_delay_secs(base, 2), 10);
        assert_eq!(retry_delay_secs(base, 3), 20);
        assert_eq!(retry_delay_secs(base, 30), 3600);
    }

    #[test]
    fn queue_name_embeds_delay() {
        assert_eq!(retry_queue_name(20), "jobs.retry.20s");
    }
}
