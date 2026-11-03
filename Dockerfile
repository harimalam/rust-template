# syntax=docker/dockerfile:1.7
#
# Multi-stage build: dependency layer cached with cargo-chef, tiny distroless runtime.
# One image runs every process: `serve` (default), `worker`, `scheduler`, `migrate`.
#
#   docker build -t rust-template-app .
#   docker run --env-file .env -p 8080:8080 rust-template-app serve

ARG RUST_VERSION=1

# ── chef: toolchain + cargo-chef ──────────────────────────────────────────────
FROM rust:${RUST_VERSION}-bookworm AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /app

# ── planner: compute the dependency recipe ────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── builder: compile deps (cached), then the app ──────────────────────────────
FROM chef AS builder
# Queries are checked against the committed .sqlx metadata, so no database is needed to build.
ENV SQLX_OFFLINE=true
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin app

# ── runtime: glibc + CA certs, no shell, non-root ─────────────────────────────
# Must match the builder's Debian release (bookworm = debian12) for glibc compatibility.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
COPY --from=builder /app/target/release/app /app
USER nonroot:nonroot
ENV HTTP_HOST=0.0.0.0 HTTP_PORT=8080
EXPOSE 8080
# Only meaningful for `serve`; compose disables it for worker / scheduler / migrate.
HEALTHCHECK --interval=15s --timeout=5s --start-period=15s --retries=3 CMD ["/app", "healthcheck"]
ENTRYPOINT ["/app"]
CMD ["serve"]
