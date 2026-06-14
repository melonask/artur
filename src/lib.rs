//! `artur` is a universal config-driven Rust HTTP gateway.
//!
//! It reads shared root configuration such as `[log]`, `[runtime]`, `[http]`,
//! `[stores.*]`, and `[transports.*]`, while Artur-owned endpoints and tasks
//! live only under `[artur]`. This lets the same `Config.toml` be mounted into
//! `artur`, `bria`, `ladon`, `oracles`, and `pano`; each package reads its own
//! namespace and shared profiles.
//!
//! Artur maps TOML-defined HTTP endpoints to static JSON responses, local
//! allowlisted tasks, in-memory async task lookup, and dependency-aware workflows
//! that can combine local tasks, database operations, HTTP service calls, and
//! response rendering.

pub mod api;
pub mod config;
pub mod error;
pub mod process;
pub mod security;
pub mod server;
pub mod store;
pub mod workflow;

pub use config::{AppConfig, load_config};
pub use server::build_router;
