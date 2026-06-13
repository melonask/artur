//! `artur` is a config-driven Rust HTTP server.
//!
//! It maps TOML-defined HTTP endpoints to a small set of generic actions:
//! static JSON responses, synchronous/asynchronous process execution, and
//! in-memory job lookup. Domain-specific work such as challenge creation,
//! wallet provisioning, Python scripts, Rust CLIs, or `npx` tools belongs in
//! configured external processes rather than in the core server.

pub mod api;
pub mod config;
pub mod error;
pub mod process;
pub mod server;

pub use config::{AppConfig, load_config};
pub use server::build_router;
