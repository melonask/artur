# Operations

## Safe operating sequence

1. Keep the TOML file and referenced executables under operator control.
2. Set required secret environment variables in the service manager, not inline in TOML.
3. Run `artur --config /protected/Config.toml check` after every change.
4. Start `artur --config /protected/Config.toml` only after validation succeeds.
5. Bind to loopback unless a reverse proxy is intentionally used.

## Limits and shutdown

Set a server `body_limit_bytes`, then use a smaller endpoint `body_limit_bytes` for upload metadata routes. Endpoint `timeout_ms` covers concurrency, rate limiting, guards, idempotency, and action execution.

On a termination signal, Artur drains active work until `runtime.shutdown_timeout_secs` expires. An expiry ends the process with an error rather than silently claiming a graceful shutdown.

## Storage

SQLite is appropriate for one persistent local instance. Artur applies its configured SQLite busy timeout to idempotency operations so concurrent writers wait briefly rather than immediately failing with a busy error. Use PostgreSQL for shared idempotency or rate limits across replicas.
