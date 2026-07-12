# Artur agent operating guide

## Prerequisites

- Rust 1.97 or newer and Cargo.
- The Artur binary built or installed (`cargo run --` is safe for repository work).
- A reviewed TOML configuration and every executable referenced by `[[artur.tasks]]` available to the service user.
- Required `${NAME}` values set in the environment. Never print, commit, or place secret values inline in TOML.

## Safe invocation and configuration workflow

1. Read the configuration and treat it as trusted operator input: task commands, arguments, working directories, store URLs, and outbound workflow URLs can execute or reach external systems.
2. Use a protected local path. Do not fetch an unreviewed remote configuration URL.
3. Validate without side effects:

   ```bash
   cargo run -- --config /protected/Config.toml check
   ```

   Validation parses TOML, expands required environment references, verifies route and dependency references, and does not bind a port, connect to stores, or start tasks.
4. Start only after validation succeeds:

   ```bash
   cargo run -- --config /protected/Config.toml
   ```

5. Prefer `127.0.0.1` binding and put TLS/network policy at a trusted reverse proxy. Configure trusted proxy CIDRs before using `{{client.ip}}` from forwarding headers.

## Validation and response semantics

- Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` after code changes.
- `400` means malformed input or a required request condition; `409` is an in-flight idempotency key; `413` is a body limit; `415` is a content-type rejection; `422` is an idempotency fingerprint mismatch; `429` is a protection limit; and `504` is an endpoint timeout.
- Error bodies contain `error` and `message`. Do not treat a successful HTTP status as evidence that a task's domain action was correct; inspect the configured task or workflow output.

## Guardrails and recovery

- Do not change a configuration to bypass API-key, challenge, x402, idempotency, rate-limit, body-limit, or concurrency controls.
- Do not execute task commands merely to inspect them. Use `check` first and request operator approval for any action with external side effects.
- Keep idempotency and rate-limit stores persistent. Use PostgreSQL for state shared by multiple replicas.
- If validation fails, correct the reported configuration issue and rerun `check`; do not start Artur with an invalid file.
- If startup fails after a configuration change, stop retry loops, preserve the error output, restore the last validated configuration, validate it, and then restart. If a shutdown timeout is reached, investigate active tasks and store contention before restarting.
