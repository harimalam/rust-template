//! Valkey (Redis protocol) wrapper: JSON cache-aside helpers, fixed-window counters for
//! rate limiting, and a small distributed lock for cron jobs.
//!
//! Cache failures never fail a request: reads degrade to a miss and writes are best-effort.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use redis::{
    Script,
    aio::{ConnectionManager, ConnectionManagerConfig},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::config::CacheSettings;

/// INCR the counter; start the window on first hit. Returns {count, ttl_ms}.
const INCR_WINDOW: &str = r"
local c = redis.call('INCR', KEYS[1])
if c == 1 then redis.call('PEXPIRE', KEYS[1], ARGV[1]) end
local ttl = redis.call('PTTL', KEYS[1])
if ttl < 0 then redis.call('PEXPIRE', KEYS[1], ARGV[1]); ttl = tonumber(ARGV[1]) end
return {c, ttl}
";

/// Delete the key only if it still holds our token (safe lock release).
const RELEASE_LOCK: &str = r"
if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end
";

/// Opens for a fixed window after a connection-level failure so a dead Valkey costs ~0 ms per
/// request (after the first failure) instead of a timeout on every cache call.
#[derive(Debug, Default)]
struct CircuitBreaker {
    open_until_ms: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl CircuitBreaker {
    fn is_open(&self) -> bool {
        now_ms() < self.open_until_ms.load(Ordering::Relaxed)
    }

    /// Returns true if this call opened a closed breaker (so the caller logs once, not per request).
    fn trip(&self, window: Duration) -> bool {
        let now = now_ms();
        let was_open = self
            .open_until_ms
            .swap(now + window.as_millis() as u64, Ordering::Relaxed)
            > now;
        !was_open
    }
}

#[derive(Clone)]
pub struct Cache {
    conn: ConnectionManager,
    prefix: Arc<str>,
    default_ttl: Duration,
    op_timeout: Duration,
    circuit_open: Duration,
    breaker: Arc<CircuitBreaker>,
}

#[derive(Debug, Clone, Copy)]
pub struct WindowHit {
    pub count: u64,
    pub retry_after_secs: u64,
}

impl Cache {
    pub async fn connect(settings: &CacheSettings) -> anyhow::Result<Self> {
        let client = redis::Client::open(settings.url.as_str()).context("invalid VALKEY_URL")?;
        // Short, bounded timeouts: the library defaults retry for many seconds per command.
        let config = ConnectionManagerConfig::new()
            .set_connection_timeout(Some(settings.op_timeout))
            .set_response_timeout(Some(settings.op_timeout))
            .set_number_of_retries(2)
            .set_max_delay(Duration::from_secs(1));
        let conn = tokio::time::timeout(
            Duration::from_secs(5),
            ConnectionManager::new_with_config(client, config),
        )
        .await
        .context("timed out connecting to Valkey")?
        .context("failed to connect to Valkey")?;
        Ok(Self {
            conn,
            prefix: Arc::from(settings.key_prefix.as_str()),
            default_ttl: settings.default_ttl,
            op_timeout: settings.op_timeout,
            circuit_open: settings.circuit_open,
            breaker: Arc::default(),
        })
    }

    /// Runs one cache command with a hard timeout and the circuit breaker.
    async fn guarded<T>(
        &self,
        op: impl Future<Output = redis::RedisResult<T>>,
    ) -> anyhow::Result<T> {
        if self.breaker.is_open() {
            anyhow::bail!("cache unavailable (circuit open)");
        }
        match tokio::time::timeout(self.op_timeout, op).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => {
                if err.is_io_error()
                    || err.is_connection_dropped()
                    || err.is_connection_refusal()
                    || err.is_timeout()
                {
                    self.trip();
                }
                Err(err.into())
            }
            Err(_) => {
                self.trip();
                anyhow::bail!("cache operation timed out after {:?}", self.op_timeout)
            }
        }
    }

    fn trip(&self) {
        if self.breaker.trip(self.circuit_open) {
            tracing::warn!(
                skip_secs = self.circuit_open.as_secs(),
                "Valkey unreachable; bypassing the cache until it recovers"
            );
        }
    }

    /// Namespaced key, e.g. `app:notes:<owner>:<id>`.
    pub fn key(&self, suffix: &str) -> String {
        format!("{}:{suffix}", self.prefix)
    }

    /// Readiness probe. Deliberately bypasses the circuit breaker so it reports the truth.
    pub async fn ping(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        let pong: String =
            tokio::time::timeout(self.op_timeout, redis::cmd("PING").query_async(&mut conn))
                .await
                .context("PING timed out")??;
        anyhow::ensure!(pong == "PONG", "unexpected PING reply: {pong}");
        Ok(())
    }

    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let mut conn = self.conn.clone();
        let raw: Option<String> = match redis::cmd("GET")
            .arg(self.key(key))
            .query_async(&mut conn)
            .await
        {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(%err, key, "cache read failed; treating as miss");
                return None;
            }
        };
        match serde_json::from_str(&raw?) {
            Ok(v) => Some(v),
            Err(err) => {
                tracing::warn!(%err, key, "cache entry undecodable; treating as miss");
                None
            }
        }
    }

    pub async fn set_json<T: Serialize>(&self, key: &str, value: &T, ttl: Option<Duration>) {
        let ttl = ttl.unwrap_or(self.default_ttl);
        let Ok(payload) = serde_json::to_string(value) else {
            return;
        };
        let mut conn = self.conn.clone();
        let result = self
            .guarded(
                redis::cmd("SET")
                    .arg(self.key(key))
                    .arg(payload)
                    .arg("PX")
                    .arg(ttl.as_millis() as u64)
                    .query_async::<()>(&mut conn),
            )
            .await;
        if let Err(err) = result {
            tracing::debug!(%err, key, "cache write failed");
        }
    }

    pub async fn delete(&self, key: &str) {
        let mut conn = self.conn.clone();
        let result: redis::RedisResult<()> = redis::cmd("DEL")
            .arg(self.key(key))
            .query_async(&mut conn)
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, key, "cache invalidation failed");
        }
    }

    /// Atomically increments a fixed-window counter. Errors bubble up so the caller can choose
    /// fail-open (rate limiting) or fail-closed behaviour.
    pub async fn hit_window(&self, key: &str, window: Duration) -> anyhow::Result<WindowHit> {
        let mut conn = self.conn.clone();
        let script = Script::new(INCR_WINDOW);
        let mut invocation = script.key(self.key(key));
        invocation.arg(window.as_millis() as u64);
        let (count, ttl_ms): (u64, i64) = self.guarded(invocation.invoke_async(&mut conn)).await?;
        Ok(WindowHit {
            count,
            retry_after_secs: (ttl_ms.max(0) as u64).div_ceil(1000).max(1),
        })
    }

    /// Current counter value without incrementing (0 if absent) and its remaining TTL in seconds.
    pub async fn peek_counter(&self, key: &str) -> anyhow::Result<(u64, u64)> {
        let mut conn = self.conn.clone();
        let k = self.key(key);
        let count: Option<u64> = self
            .guarded(redis::cmd("GET").arg(&k).query_async(&mut conn))
            .await?;
        let ttl_ms: i64 = self
            .guarded(redis::cmd("PTTL").arg(&k).query_async(&mut conn))
            .await?;
        Ok((count.unwrap_or(0), (ttl_ms.max(0) as u64).div_ceil(1000)))
    }

    pub async fn delete_strict(&self, key: &str) -> anyhow::Result<()> {
        let mut conn = self.conn.clone();
        redis::cmd("DEL")
            .arg(self.key(key))
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    /// Tries to take a named lock for `ttl`. Returns `None` if somebody else holds it.
    pub async fn try_lock(&self, name: &str, ttl: Duration) -> anyhow::Result<Option<LockGuard>> {
        let mut conn = self.conn.clone();
        let key = self.key(&format!("lock:{name}"));
        let token = uuid::Uuid::now_v7().to_string();
        let acquired: Option<String> = self
            .guarded(
                redis::cmd("SET")
                    .arg(&key)
                    .arg(&token)
                    .arg("NX")
                    .arg("PX")
                    .arg(ttl.as_millis() as u64)
                    .query_async(&mut conn),
            )
            .await?;
        Ok(acquired.map(|_| LockGuard {
            conn: self.conn.clone(),
            key,
            token,
        }))
    }
}

/// Held lock. Call [`LockGuard::release`] when done; otherwise it expires with its TTL.
pub struct LockGuard {
    conn: ConnectionManager,
    key: String,
    token: String,
}

impl LockGuard {
    pub async fn release(mut self) {
        let result: redis::RedisResult<i64> = Script::new(RELEASE_LOCK)
            .key(&self.key)
            .arg(&self.token)
            .invoke_async(&mut self.conn)
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, key = %self.key, "failed to release lock; it will expire");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_opens_once_and_closes_after_the_window() {
        let breaker = CircuitBreaker::default();
        assert!(!breaker.is_open());
        assert!(
            breaker.trip(Duration::from_millis(80)),
            "first trip reports the transition"
        );
        assert!(breaker.is_open());
        assert!(
            !breaker.trip(Duration::from_millis(80)),
            "re-tripping while open is not a new transition"
        );
        std::thread::sleep(Duration::from_millis(120));
        assert!(
            !breaker.is_open(),
            "closes by itself so the next call probes Valkey again"
        );
    }
}
