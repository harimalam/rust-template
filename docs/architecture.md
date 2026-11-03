# Architecture

A **modular monolith**: one codebase and one binary, organised by business feature, with four run modes
(`serve`, `worker`, `scheduler`, `migrate`). It starts simple and has clear seams for splitting later.

## Layers

```
 HTTP request
     │
     ▼
 ┌─────────────────────────── bootstrap/router.rs ───────────────────────────┐
 │ request-id → panic catch → trace → metrics → timeout → gzip → CORS →      │
 │ security headers → body limit                                              │
 └──────────────────────────────────┬────────────────────────────────────────┘
                                    ▼
 modules/<feature>/handlers.rs   extract + validate input, authorise (AuthUser::require),
                                 call the service, wrap result in the response envelope
                                    │
 modules/<feature>/service.rs    business rules, ownership checks, cache-aside, publishing jobs
                                    │
 modules/<feature>/repository.rs SQL only (sqlx::query_as!), no rules
                                    │
 infra/*                         adapters for Postgres, Valkey, SMTP, S3, RabbitMQ
```

Rules that keep the structure honest:

1. **Handlers contain no business logic.** They translate HTTP ⇄ service calls. Permission checks sit here because
   they are part of the HTTP boundary (`user.require(perm::NOTES_WRITE)?`).
2. **Services never see HTTP types** (no `StatusCode`, no extractors). They return `AppResult<T>`.
3. **Repositories contain SQL and nothing else.** Ownership scoping (`WHERE owner_id = $2`) lives in the SQL so a
   forgotten check can't leak rows.
4. **External systems sit behind traits** (`Mailer`, `ObjectStorage`, `JobPublisher`) held in `AppState` as `Arc<dyn …>`,
   so tests swap in fakes. Valkey and Postgres are used directly: faking them would test the fake, not the behaviour.
5. **`AppError` is the only place** that picks HTTP status codes and the public error shape. Internal errors are logged in
   full and returned as a generic message.

Traits use `async_trait` because they are used as `dyn` objects; everything else uses native `async fn`.

## Request lifecycle

1. `request_id_middleware` accepts a sane inbound `x-request-id` (≤128 chars of `[A-Za-z0-9._-]`) or generates a UUIDv7,
   stores it in a task-local, and echoes it on the response. The tracing span, error bodies and enqueued jobs all read it,
   so one ID follows a request from the log line through the queue to the worker's log line.
2. Auth routes pass through `limit_auth` (Valkey `INCR`+`PEXPIRE` in one Lua script — atomic across replicas).
3. `AuthUser` verifies the bearer JWT (HS256, issuer + expiry checked) and exposes `id`, `roles`, `permissions`.
4. `ValidatedJson<T>` deserialises and runs `validator` rules; failures become `422 validation_failed` with per-field details.
5. The service runs; errors bubble as `AppError`; `IntoResponse` renders the envelope.
6. `track_http_metrics` records count + latency labelled by *route template*.

## Authentication model

```
register ─► user row + 'user' role (one transaction) ─► verification token (hashed) ─► SendEmail job ─► worker ─► SMTP
login    ─► lockout check ─► Argon2id verify (dummy hash if email unknown) ─► verified/active check
          ─► access JWT (15 min, roles+perms inside) + refresh token (random, SHA-256 stored, family id)
refresh  ─► lock row (FOR UPDATE) ─► revoked? ──yes──► revoke whole family, 401
                                      └─no─► mark old revoked(replaced_by) + insert new, same family
```

- Opaque refresh tokens (not JWTs) so they can be revoked and rotated; only hashes are stored.
- `FOR UPDATE` serialises concurrent refreshes with the same token: exactly one wins, the other is treated as reuse.
- Password reset consumes a single-use token, sets the password, marks the email verified (the user proved inbox
  access) and revokes every refresh token.
- RBAC tables: `roles`, `permissions`, `role_permissions`, `user_roles`. Roles and flattened permissions are put
  in the access token at issue time, so authorisation needs no query. Trade-off: changes apply when the token is
  next issued (≤ access TTL).

## Data

- UUIDv7 primary keys (time-ordered → index-friendly, no coordination).
- `TIMESTAMPTZ` everywhere; audit columns `created_at/updated_at/created_by/updated_by` on user-owned data.
- **Soft delete** (`deleted_at`) for notes/files/users with *partial indexes* `WHERE deleted_at IS NULL` so live-row
  queries stay fast; the scheduler hard-deletes after `SOFT_DELETE_RETENTION_DAYS`.
- `email` is `CITEXT` with a unique index over live rows, so addresses are case-insensitive and reusable after deletion.
- Dynamic ORDER BY uses a per-resource **column whitelist** (`SORTABLE`) mapped to static strings; user input never reaches SQL text.
  Everything else is parameterised.
- Migrations are plain SQL files embedded with `sqlx::migrate!`; `.sqlx/` holds query metadata so CI/Docker builds don't need a database.

## Caching

`infra/cache.rs` wraps one multiplexed `ConnectionManager`.

- **Cache-aside** for `GET /notes/{id}`: key `…:notes:{owner_id}:{id}`, TTL `CACHE_DEFAULT_TTL_SECS`; updates and deletes call
  `delete` on the key. The owner is in the key *and* re-checked after a hit.
- Reads that fail or can't be decoded are misses; writes are best-effort. The cache is an optimisation, never a dependency
  for correctness.
- **Bounded latency when Valkey is down.** Left alone, the redis client retries with exponential backoff, which turned each
  request into 9–29 s (several cache calls per request × several retries). So: the connection manager uses short timeouts and
  2 retries; every call is wrapped in `tokio::time::timeout(CACHE_OP_TIMEOUT_MS)`; and a **circuit breaker** opens for
  `CACHE_CIRCUIT_OPEN_SECS` after a connection-level failure, making later calls fail instantly (the transition is logged once).
  After the window, the next call probes Valkey again. `ping()` (readiness) bypasses the breaker so it always reports the truth.
- Rate limits and login-failure counters are fixed-window counters (Lua `INCR` + `PEXPIRE`). They **fail open** on Valkey errors.
- `try_lock` (`SET NX PX` + token) coordinates cron ticks. Release is compare-and-delete in Lua.

## Background jobs

```
 API/service ──publish (confirmed, persistent)──► exchange "jobs" ──► jobs.default ──► worker
                                                                          ▲              │ error
                                      TTL expires (dead-letter back) ─────┤              ▼
                                              jobs.retry.<delay>s ◄── exchange "jobs.retry"  (attempt < max)
                                                                                         │ attempt ≥ max, or undecodable
                                                                                         ▼
                                                                              exchange "jobs.dead" ─► jobs.dead
```

- Messages are JSON envelopes `{id, request_id, enqueued_at, job:{type,…}}`; the attempt count is an `x-attempts` header.
- **Retry delays use one queue per delay** (`x-message-ttl` fixed per queue). A single queue with per-message TTL would let a
  long-delay message block shorter ones behind it, because RabbitMQ only expires from the head. Delay is in the queue name so
  config changes don't hit AMQP's "queue arguments differ" error.
- Acks are manual and happen only after the retry/dead-letter publish is **confirmed**; if re-publishing fails the message stays
  unacked and the broker redelivers it. At-least-once delivery: **make job handlers idempotent**.
- Concurrency is bounded by `RABBITMQ_PREFETCH`. On shutdown the consumer is cancelled, in-flight jobs finish (bounded by
  `SHUTDOWN_TIMEOUT_SECS`), the rest are redelivered.
- If the broker connection drops, the worker exits non-zero so the orchestrator restarts it (simple and reliable). The API's
  publisher reconnects transparently.
- Registration/forgot-password enqueue the email rather than sending inline: SMTP latency or outages never affect request latency,
  and failures retry with backoff.

## Scheduler

`app scheduler` runs `tokio-cron-scheduler`. Each job execution first takes a Valkey lock named after the job with TTL
`CRON_LOCK_TTL_SECS`. The lock is intentionally **not** released on success: it expires on its own, which prevents a second replica
that ticks a few milliseconds later from re-running the job. Requirement: the TTL must be shorter than the shortest cron
interval and longer than clock skew between replicas. If Valkey is unreachable the tick is skipped (better to miss a purge than
run it twice). Run 2+ replicas for availability.

## Observability

- `tracing` for logs and spans. `LOG_FORMAT=json` emits one flattened JSON object per line including current span fields (`request_id`, `job_id`).
- `metrics` + `metrics-exporter-prometheus`: `http_requests_total`, `http_request_duration_seconds` (histogram), `rate_limited_requests_total`,
  `cron_job_runs_total`. Labels use the matched route template to bound cardinality.
- `/health/live` never touches dependencies (so an outage doesn't restart healthy pods); `/health/ready` checks database, cache,
  storage and queue in parallel with a 2 s timeout each and returns 503 if any fail, without leaking error details.

## Decisions & trade-offs

| Decision | Alternative | Why |
|---|---|---|
| SQLx raw SQL | SeaORM / Diesel | Compile-time-checked SQL with no DSL to learn; explicit queries are easy to optimise. |
| HS256 JWT | RS256/EdDSA | Single service issues and verifies. Switch to asymmetric keys if other services need to verify tokens. |
| Permissions in JWT | DB lookup per request | No query on the hot path; staleness bounded by short TTL. |
| Opaque refresh tokens | JWT refresh tokens | Revocable, rotatable, theft-detectable. |
| Separate `worker`/`scheduler` processes | In-process tasks | Scale and restart independently; API replicas stay timer-free. |
| Fail-open rate limiting | Fail-closed | A cache outage shouldn't lock everyone out of login; lockout and Argon2 cost still apply. |
| Fixed-window limiter | Sliding window / token bucket | One atomic Lua call, easy to reason about; allows bursts at window edges. Swap if that matters. |
| Distroless runtime | Alpine/Debian slim | No shell or package manager in the image; glibc + CA certs only. |
| Migrations in the binary | Files in the image | One artefact, version-locked schema; `app migrate` runs them as a release step. |

## Scaling notes

- Every process is stateless: run N API pods behind any load balancer; rate limits, locks and caches live in Valkey.
- The Postgres pool is per process (`DATABASE_MAX_CONNECTIONS`); size it against `max_connections / replicas`, or put PgBouncer in front.
- Throughput of background work scales with worker replicas × `RABBITMQ_PREFETCH`.
- Heavy CPU work (Argon2) runs on Tokio's blocking pool so it can't stall request handling.
