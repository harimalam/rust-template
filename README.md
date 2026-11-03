# Rust Backend Template

A production-oriented backend starter: **Rust + Axum + PostgreSQL**, with Valkey caching, SMTP email,
S3-compatible object storage (RustFS), RabbitMQ background jobs and cron scheduling — all configured
through environment variables and runnable with Docker.

Clone it, run three commands, and you have a working API with authentication, RBAC, an example CRUD
module, file uploads, background email delivery, metrics and OpenAPI docs.

## Contents

- [Quick start](#quick-start)
- [What's included](#whats-included)
- [Stack and why](#stack-and-why)
- [Running](#running)
- [Configuration](#configuration)
- [API tour](#api-tour)
- [Project layout](#project-layout)
- [Adding a feature](#adding-a-feature)
- [Testing](#testing)
- [Security notes](#security-notes)
- [Deployment](#deployment)
- [Not included (extension points)](#not-included-extension-points)

## Quick start

Requirements: Rust (stable), Docker with Compose, and [`sqlx-cli`](https://crates.io/crates/sqlx-cli)
for development (`cargo install sqlx-cli --no-default-features --features rustls,postgres`).

```bash
cp .env.example .env     # every setting is documented in this file
make up                  # postgres, valkey, rabbitmq, rustfs, mailpit
make migrate             # apply database migrations
make dev                 # API on http://localhost:8080
```

In two more terminals run `make worker` (sends emails) and `make scheduler` (cron jobs).

| URL | What |
|---|---|
| http://localhost:8080/docs | OpenAPI / Scalar API reference |
| http://localhost:8080/health/ready | Readiness (database, cache, storage, queue) |
| http://localhost:8080/metrics | Prometheus metrics |
| http://localhost:8025 | Mailpit — every email the app sends lands here |
| http://localhost:15672 | RabbitMQ management (`app` / `app_password`) |
| http://localhost:9001 | RustFS console (`rustfsadmin` / `rustfsadmin-secret`) |

Create the first administrator (there is intentionally no public endpoint for this):

```bash
ADMIN_PASSWORD='a-long-passphrase' cargo run -- create-admin --email admin@example.com
```

## What's included

| Area | Details |
|---|---|
| **Auth** | Register, login, JWT access tokens, rotating opaque refresh tokens with reuse detection, logout, email verification, forgot/reset password. Argon2id hashing. |
| **Authorization** | RBAC: roles → permissions (`notes:write`, `users:manage`, …) stored in Postgres, embedded in the access token, enforced with `user.require(perm::…)`. |
| **Abuse protection** | Per-IP rate limiting on `/auth/*` (shared across replicas via Valkey), per-account login lockout, equalised-cost failure path (unknown email costs the same Argon2 work as a wrong password), no account enumeration on reset/resend. |
| **API conventions** | `/api/v1`, uniform `{data, meta}` / `{error}` envelopes, validated JSON, pagination + whitelisted sorting + search, request/correlation IDs, OpenAPI. |
| **Database** | SQLx with compile-time-checked queries, embedded migrations, pooled connections, transactions, soft deletes, audit columns, partial indexes. |
| **Cache** | Valkey wrapper: cache-aside helpers, fixed-window counters, distributed lock. Every call has a hard timeout plus a circuit breaker, so a Valkey outage degrades to cache misses at normal latency instead of failing or stalling requests. |
| **Jobs** | RabbitMQ with publisher confirms, persistent messages, exponential-backoff retry queues, dead-letter queue, graceful drain on shutdown. |
| **Cron** | `tokio-cron-scheduler`; each tick takes a Valkey lock so multiple replicas run a job once. |
| **Email** | SMTP via `lettre` (plain, STARTTLS or TLS); HTML + text templates; sent by the worker, never inline in a request. |
| **Storage** | S3 API (RustFS/MinIO/AWS): multipart upload with size limit, MIME allow-list **and** magic-byte sniffing, server-generated keys, presigned downloads. |
| **Observability** | Structured JSON logs (pretty in dev), request IDs in logs/errors/jobs, Prometheus metrics by route template, liveness/readiness probes. |
| **Security** | Secure headers, CORS allow-list, body-size and timeout limits, panic catching, secrets only from env, startup config validation. |
| **Ops** | Multi-stage Dockerfile (distroless, non-root), Compose, GitHub Actions (fmt, clippy, tests + coverage, cargo-deny, image build), graceful shutdown. |

## Stack and why

**Rust 2024 + Axum 0.8 + Tokio.** Evaluated against Go, TypeScript (NestJS/Fastify), Java (Spring Boot),
Python (FastAPI) and C# (ASP.NET Core). Rust wins here on memory footprint (a few tens of MB per process),
predictable latency (no GC), and compile-time guarantees — invalid states, missing `match` arms and even
SQL typos fail the build rather than production. The costs are slower compiles and a steeper learning
curve; the template exists to absorb most of that up-front.

| Choice | Why |
|---|---|
| Axum | Tower middleware ecosystem, first-class Tokio, minimal magic. |
| SQLx | Plain SQL, checked against the real schema at compile time, built-in migrations, no ORM surprises. |
| Valkey (`redis` crate) | Open-source Redis-protocol server; swap in Redis/KeyDB with no code change. |
| lapin | Mature async AMQP 0-9-1 client for RabbitMQ. |
| aws-sdk-s3 | Works with any S3-compatible store; presigning built in. |
| rustls + aws-lc-rs everywhere | One TLS stack, no OpenSSL to patch or link. `deny.toml` bans `openssl-sys`/`native-tls`. |
| Modular monolith | One deployable binary with feature modules and four run modes. Split a module into a service only when you must. |

## Running

### Local development (app on your machine, infra in Docker)

```bash
make up && make migrate
make dev            # or: cargo run -- serve
make worker         # cargo run -- worker
make scheduler      # cargo run -- scheduler
```

### Everything in Docker

```bash
docker compose --profile app up --build -d
```

Starts infra plus `migrate` (one-shot), `api`, `worker` and `scheduler` from one image. The compose file
overrides hostnames (`postgres`, `valkey`, …) for in-network use, so the same `.env` works for both modes.

### One binary, several commands

```
app serve         HTTP API
app worker        RabbitMQ job consumer
app scheduler     cron jobs
app migrate       apply migrations and exit
app create-admin  bootstrap the first administrator
app healthcheck   exit 0 if /health/live answers (used by Docker HEALTHCHECK)
```

### Make targets

`up` `down` `dev` `worker` `scheduler` `migrate` `migration name=…` `sqlx-prepare` `fmt` `lint` `test` `coverage` `docker-build` `hooks` (enable the pre-commit hook).

## Configuration

All configuration is environment variables, validated at startup. Missing or malformed values are
reported **together** and the process exits with status 2:

```
invalid configuration (3 problem(s)):
  - DATABASE_URL is required
  - HTTP_PORT="abc" is invalid: invalid digit found in string
  - JWT_SECRET must be at least 32 bytes (try: openssl rand -base64 48)
```

[`.env.example`](.env.example) documents every variable. Highlights:

| Group | Variables |
|---|---|
| App | `APP_ENV` (development/staging/production), `HTTP_*`, `LOG_LEVEL`, `LOG_FORMAT`, `CORS_ALLOWED_ORIGINS`, `TRUST_PROXY_HEADERS`, `DOCS_ENABLED` |
| Postgres | `DATABASE_URL`, `DATABASE_MAX_CONNECTIONS`, `RUN_MIGRATIONS_ON_START` |
| Valkey | `VALKEY_URL`, `CACHE_KEY_PREFIX`, `CACHE_DEFAULT_TTL_SECS`, `CACHE_OP_TIMEOUT_MS`, `CACHE_CIRCUIT_OPEN_SECS` |
| Auth | `JWT_SECRET`, `JWT_*_TTL_SECS`, `REQUIRE_EMAIL_VERIFICATION`, `RATE_LIMIT_AUTH_*`, `LOGIN_*` |
| SMTP | `SMTP_HOST/PORT/USERNAME/PASSWORD/TLS/FROM` |
| S3 | `S3_ENDPOINT`, `S3_PUBLIC_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY/SECRET_KEY`, `MAX_UPLOAD_BYTES`, `UPLOAD_ALLOWED_MIME_TYPES` |
| RabbitMQ | `RABBITMQ_URL`, `RABBITMQ_PREFETCH`, `JOB_MAX_ATTEMPTS`, `JOB_RETRY_BASE_DELAY_SECS` |
| Cron | `CRON_*` (6-field expressions), `CRON_LOCK_TTL_SECS`, `SOFT_DELETE_RETENTION_DAYS` |

Production guards: `APP_ENV=production` refuses the example `JWT_SECRET` and wildcard CORS, defaults to
JSON logs, enables HSTS and turns the docs UI off.

`S3_PUBLIC_ENDPOINT` is the address embedded in presigned URLs — it must be reachable by *clients*, which
inside Docker differs from `S3_ENDPOINT` (the in-network address the app uses).

## API tour

Full reference at `/docs`. A typical session:

```bash
B=http://localhost:8080/api/v1

curl -s -X POST $B/auth/register -H 'content-type: application/json' \
  -d '{"email":"ada@example.com","password":"correct horse battery staple","full_name":"Ada"}'

# Open http://localhost:8025, copy the token from the verification email:
curl -s -X POST $B/auth/verify-email -H 'content-type: application/json' -d '{"token":"<token>"}'

TOKEN=$(curl -s -X POST $B/auth/login -H 'content-type: application/json' \
  -d '{"email":"ada@example.com","password":"correct horse battery staple"}' | jq -r .data.access_token)

curl -s -X POST $B/notes -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"title":"Hello","body":"first note"}'
curl -s "$B/notes?q=hello&sort=-created_at&per_page=10" -H "authorization: Bearer $TOKEN"
curl -s -X POST $B/files -H "authorization: Bearer $TOKEN" -F "file=@photo.png;type=image/png"
```

Responses:

```jsonc
// success                                    // error
{ "data": { ... },                            { "error": {
  "meta": { "page": 1, "per_page": 20,           "code": "validation_failed",
            "total": 42, "total_pages": 3 } }    "message": "validation failed",
                                                 "details": [{ "field": "email", "message": "..." }],
                                                 "request_id": "01a1a0a3-…" } }
```

Status codes: `400` bad input · `401` unauthenticated · `403` forbidden · `404` not found (also used for
other users' resources) · `409` conflict · `413` too large · `415` bad media type · `422` validation ·
`429` rate limited (with `Retry-After`) · `503` dependency down (readiness).

## Project layout

```
src/
├── main.rs              CLI entry (clap): serve | worker | scheduler | migrate | create-admin | healthcheck
├── lib.rs               library root (tests build the router from here)
├── config/              typed settings, env parsing, startup validation
├── bootstrap/           AppState, router + middleware stack, server, shutdown, telemetry, OpenAPI
├── http/                cross-cutting HTTP: AppError, response envelope, pagination, extractors, middleware
├── infra/               adapters: db pool, cache, mail, storage, queue (topology/publisher/consumer)
├── modules/             business features, one folder each
│   ├── auth/            handlers · service · repository · dto · jwt · password · tokens · rbac · emails
│   ├── users/           profile + admin user management (RBAC demo)
│   ├── notes/           ★ example CRUD — copy this to start a new feature
│   ├── files/           S3 uploads
│   └── health/          liveness / readiness
├── jobs/                job definitions + handlers (RabbitMQ payloads)
└── scheduler/           cron job registry
migrations/              SQL migrations (embedded into the binary)
.sqlx/                   committed query metadata → builds without a database
tests/                   API/integration tests + shared harness (tests/common)
docs/architecture.md     design notes and trade-offs
```

Dependency rule: `handlers → service → repository`. Handlers only extract, authorise and shape
responses; services hold business rules; repositories hold SQL. See [docs/architecture.md](docs/architecture.md).

## Adding a feature

Copy `src/modules/notes/` and:

1. Write a migration: `make migration name=create_widgets` (use `-r` for reversible).
2. Add `domain.rs` (entity), `dto.rs` (request/response + `#[derive(Validate, ToSchema)]`).
3. `repository.rs`: SQL with `sqlx::query_as!` (dynamic sort/filter uses `QueryBuilder` + a column whitelist).
4. `service.rs`: ownership/business rules, cache invalidation, publishing jobs via `state.jobs`.
5. `handlers.rs`: extract → `user.require(perm::WIDGETS_WRITE)` → call service; annotate with `#[utoipa::path]`.
6. Mount in `bootstrap/router.rs`, list the handlers in `bootstrap/openapi.rs`.
7. Add permissions in the migration (`INSERT INTO permissions …`, `role_permissions …`) and constants in `modules/auth/rbac.rs`.
8. `make sqlx-prepare` and commit `.sqlx/`.

New background job: add a variant to `jobs::Job`, handle it in `jobs/handler.rs`, publish with `state.jobs.publish(...)`.
New cron job: write an `async fn(Arc<SchedulerContext>) -> anyhow::Result<String>` in `scheduler/mod.rs` and register it.

## Testing

```bash
make up          # tests need Postgres + Valkey
make test        # 58 tests: unit + API/integration
make coverage    # requires cargo-llvm-cov
```

- Each `#[sqlx::test]` gets its own migrated Postgres database — tests are isolated and run in parallel.
- Valkey is real; every test app uses a unique `CACHE_KEY_PREFIX`.
- Mail, queue and object storage use in-memory fakes behind the production traits (`tests/common`).
  Requests go through the **full** middleware stack via `tower::ServiceExt::oneshot`.
- Covered: auth flows (verify, lockout, rotation + theft detection, reset), RBAC, ownership isolation
  (including cache hits), caching/invalidation, pagination/sort/search, upload validation, health,
  request IDs, security headers, CORS, body limits, OpenAPI coverage.

The RabbitMQ and S3 adapters themselves are exercised by running the stack (`docker compose --profile app up`),
not by the fake-based tests.

## Security notes

- **Secrets** come only from the environment. `.env` is git-ignored. Generate `JWT_SECRET` with `openssl rand -base64 48`.
- **Access-token staleness**: roles/permissions live in the JWT, so role changes apply on next refresh/login
  (≤ `JWT_ACCESS_TTL_SECS`, default 15 min). Lower the TTL if that is too long.
- **Refresh tokens** are random, stored as SHA-256 hashes, rotated on use. Presenting a used token revokes the whole session family.
- **Login lockout is per email** (temporary), which stops targeted guessing but lets an attacker briefly lock a known address.
  Per-IP limits sit on top of it.
- **Registration returns 409** for existing emails (usability over enumeration resistance). Forgot-password and
  resend-verification always return 200.
- **Rate limiting fails open** if Valkey is unreachable (logged), so a cache outage doesn't block logins. Measured with Valkey stopped: requests stay at ~0.05–0.45 s (the first call after the outage pays one 300 ms timeout, then the breaker skips the cache for `CACHE_CIRCUIT_OPEN_SECS`), `/health/ready` returns 503, and the cache resumes by itself on recovery.
- **Behind a proxy** set `TRUST_PROXY_HEADERS=true` so limits key on the real client IP. This trusts the *last*
  `X-Forwarded-For` hop and assumes exactly one trusted proxy. Never enable it when the API is directly exposed.
- **`/metrics` is unauthenticated** — block it at your ingress or scrape it only from the internal network.
- **Dead-letter queue** (`jobs.dead`) holds full job payloads, including email bodies with one-time tokens.
  Restrict RabbitMQ access and set a retention policy on that queue.
- **Uploads**: type allow-list + magic-byte check, size cap, server-generated object keys, `Content-Disposition: attachment` on downloads.
- Change the default infra credentials in `.env` for anything beyond your laptop; RustFS logs a warning for defaults.

## Deployment

- Build once (`docker build .`), run the same image as `serve`, `worker` and `scheduler`; scale each independently.
  All three are stateless.
- Run `app migrate` as a separate release step before starting new `serve` pods (`RUN_MIGRATIONS_ON_START=false`).
  Make migrations backward-compatible so old and new versions can overlap during a rollout.
- Probes: liveness `GET /health/live`, readiness `GET /health/ready`. The container `HEALTHCHECK` uses `app healthcheck`
  (distroless has no curl).
- On SIGTERM the API stops accepting connections and drains for up to `SHUTDOWN_TIMEOUT_SECS`; the worker stops consuming
  and finishes in-flight jobs; unfinished messages are redelivered.
- Retry queues are named by delay (`jobs.retry.5s`). Changing `JOB_RETRY_BASE_DELAY_SECS` creates new queues; remove the old ones when empty.
- Production/staging: set `APP_ENV`, `LOG_FORMAT=json`, strong secrets, real SMTP (`SMTP_TLS=starttls|tls`), a managed
  Postgres/Valkey/RabbitMQ, and `S3_AUTO_CREATE_BUCKET=false` with buckets provisioned by IaC.
- Pin the `rustfs/rustfs` image tag in `compose.yaml` (it uses `latest` for convenience).

## Not included (extension points)

Deliberately left out to keep the template focused; each has an obvious seam:

- **OAuth2 / social login** — `users.password_hash` is nullable. Add a `user_identities (provider, subject, user_id)` table,
  a callback handler that finds-or-creates the user, then reuse `auth::service::issue_token_pair`.
- **OpenTelemetry export** — logs/spans already use `tracing`; add `tracing-opentelemetry` + an OTLP exporter layer in `bootstrap/telemetry.rs`.
- **Idempotency keys** — add an `Idempotency-Key` middleware storing the response in Valkey (`SET NX`) for POSTs that must not repeat.
- **Read replicas** — add `DATABASE_REPLICA_URL`, a second `PgPool` in `AppState`, and use it from read-only repository functions.
- **Kubernetes manifests** — the image, probes, env-only config and graceful shutdown are K8s-ready; add Deployments for `serve`/`worker`/`scheduler` and a migrate Job.
- **Error tracking** — add a Sentry (or similar) layer next to the tracing subscriber.
