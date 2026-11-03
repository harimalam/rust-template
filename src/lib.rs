//! Rust backend template. See `docs/architecture.md` for the layering rules.

pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod http;
pub mod infra;
pub mod jobs;
pub mod modules;
pub mod scheduler;

pub use bootstrap::{router::build_router, state::AppState};
