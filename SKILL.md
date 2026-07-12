---
name: artur
description: Use when configuring, operating, testing, debugging, or safely invoking the Artur config-driven HTTP gateway, including its TOML routes, tasks, workflows, stores, transports, endpoint protections, idempotency, rate limits, and async jobs.
---

# Artur AI operating manual

## Purpose and boundaries

Artur is a Rust HTTP gateway whose behavior is defined by a privileged TOML file. It exposes configured routes that return static JSON, run allowlisted child processes, poll in-memory async jobs, or execute workflow DAGs. Use this skill for deterministic, implementation-grounded changes and operations.

Artur is **not** a TLS terminator, identity provider, durable queue, scheduler, retry system, secret manager, sandbox, or business-domain validator. A task command, its scripts, and outbound services own domain behavior. Async job records and failure blocks are process-local and are lost on restart; neither is cross-replica state.

## Prerequisites and trust boundary

- Use Rust 1.97 or newer with Cargo for repository work. `Cargo.toml` declares the required toolchain.
- Supply `--config <PATH>` or set `ARTUR_CONFIG`. A config location may be a local path or an `http(s)` URL, but agents must use a reviewed local, access-controlled file; remote configuration is executable control-plane input.
- The service user needs access to every configured executable, working directory, SQLite path, PostgreSQL endpoint, and outbound HTTP target.
- Treat configuration as code: task commands, arguments, environment, working directories, SQL, and HTTP URLs can cause side effects. Use absolute command paths in production where feasible.
- `${NAME}` is expanded before TOML parsing and every referenced variable must exist. Keep secret values in the process environment or a deployment secret mechanism, never inline, in command arguments, logs, or commits.

## Safe decision workflow

1. Identify the requested route and every reachable action, task, store, transport, and template source. Inspect commands and SQL without executing them.
2. Establish side-effect scope: child processes, mutations, outbound calls, and persistent state. Obtain operator approval before invoking an endpoint that can have external effects.
3. Prefer a protected local config and provide all required environment variables to the Artur process.
4. Validate only:
   ```bash
   cargo run -- --config /protected/Config.toml check
   ```
   `check` loads, expands environment references, parses, and validates configuration. It does not bind a listener, connect to stores, run tasks, or contact workflow transports. A remote config URL is fetched before validation, which is why agents must not use unreviewed URLs.
5. Start only after a successful check:
   ```bash
   cargo run -- --config /protected/Config.toml
   ```
   Installed binary equivalents are exact:
   ```bash
   artur --config /protected/Config.toml check
   artur --config /protected/Config.toml
   ```
6. Verify the built-in liveness endpoint, then make the smallest approved functional request:
   ```bash
   curl --fail-with-body http://127.0.0.1:46796/healthz
   ```
7. If validation or startup fails, stop retry loops, retain the error output without secrets, restore the last validated config, rerun `check`, then restart.

## Configuration structure and resolution

`version = 1`, `[artur]`, and at least one `[[artur.endpoints]]` are required. Artur rejects unsupported schema versions. Artur-owned definitions must be under `[artur]`; root-level `[[endpoints]]`, `[[tasks]]`, and `[server]` are not accepted as Artur settings.

Reusable universal sections are root-level: `[log]`, `[runtime]`, `[paths.<id>]`, `[stores.<id>]`, and `[transports.http.<id>]`. Other root package namespaces are ignored by Artur, enabling a shared config. `RUST_LOG` takes precedence when it parses as an EnvFilter; otherwise logging uses `log.level`, defaulting to `artur=info,tower_http=info`. The effective listener defaults are `127.0.0.1:46796` with a `1048576`-byte server body limit; `[artur.server]` overrides `bind`, `port`, and `body_limit_bytes`. `runtime.shutdown_timeout_secs` controls graceful shutdown after SIGTERM or Ctrl-C. `runtime.worker_threads`, `runtime.tmp_dir`, `runtime.max_payload_bytes`, and `[paths]` are accepted shared metadata but do not change Artur request execution.

Only `sqlite` and `postgres` stores are supported. A SQLite URL accepts `sqlite://PATH`, `sqlite:PATH`, a raw path, or `:memory:`. `:memory:` is prohibited for rate-limit and idempotency stores. `connect_timeout_secs` is used as SQLite's idempotency busy timeout; it is not a general connection timeout.

HTTP transports contain `base_url`, optional default `headers`, and optional `timeout_ms`. A workflow HTTP step may use a transport, an absolute URL, or both; an absolute step URL takes precedence over the transport base URL. Transport headers are added before per-step headers.

Minimal valid static route:

```toml
version = 1

[artur.server]
bind = "127.0.0.1"
port = 46796

[[artur.endpoints]]
name = "hello"
method = "GET"
path = "/v1/hello"
action = "respond.static"

[artur.endpoints.response]
status = 200
body = { ok = true }
```

Endpoint names and method/path pairs must be unique. Methods are `GET`, `POST`, `PUT`, `PATCH`, and `DELETE`. Paths start with `/`; `{name}` and legacy `:name` path parameters are accepted. Actions are exactly `respond.static`, `task.run`, `workflow.run`, and `job.get`.

## Tasks, request context, and templates

A task has a unique name, executable `command`, optional `args`, `env`, and templated `working_dir`. Defaults are `mode = "sync"`, `inherit_env = true`, `success_exit_codes = [0]`, `timeout_ms = 30000`, output limits of 1048576 bytes each, and `stdout_format = "text"`. Set `inherit_env = false` when an allowlisted task should not inherit the service environment, then explicitly provide only needed environment values.

`stdin.type` is `none`, `body`, `request_json`, or `template` (which requires `template`). A timed-out process returns a task output with `ok: false` and `timed_out: true`; a non-success exit or invalid JSON with `stdout_format = "json"` likewise produces `ok: false`. Captured output is truncated at its configured limit and marked by `stdout_truncated` or `stderr_truncated`; it is not a process-output kill limit.

Templates render in task args, task env values, task working directories, template stdin, rate-limit and failure-block keys, API-key values, workflow SQL and parameters, workflow HTTP URL/headers/body, workflow respond values, and explicit workflow result bodies. Whole-value JSON templates retain their JSON type; embedded templates stringify JSON objects and arrays. Unknown keys render as an empty string; an unclosed `{{` expression is a configuration error. Static-response bodies/headers and workflow result headers are returned as configured and are not template-rendered.

Available values are `{{method}}`, `{{uri}}`, `{{path}}`, `{{client.ip}}`, `{{body}}`, `{{request}}`/`{{request_json}}`, `{{body_json}}`, `{{param.<name>}}`, `{{query.<name>}}`, `{{header.<lowercase-name>}}`, `{{env.<NAME>}}`, `{{body_json.<dot.path>}}`, `{{steps.<id>.<path>}}`, and `{{step.<id>.<path>}}`. Array indices work in JSON paths. The request JSON passed to a task includes method, client IP, URI, path, params, query, headers, text-decoded body, parsed JSON body when valid, and completed workflow steps. Raw non-UTF-8 body bytes are lossy-decoded for templates and task stdin.

## Endpoints, workflows, stores, and transports

- `respond.static` requires `[artur.endpoints.response]`; it returns its JSON `body`, optional `status` (default 200), and configured headers.
- `task.run` requires `task`. A synchronous task returns its `TaskOutput` JSON. An asynchronous task returns `{"job_id":"<UUID>","status":"running"}` immediately.
- `job.get` requires `{job_id}` or `:job_id` in its path. It returns the in-memory job record or `404` after an unknown ID or a restart.
- `workflow.run` requires one or more steps. Ready dependency layers run in parallel; a step only sees outputs from prior layers, never same-layer outputs. Cycles and unknown dependencies are rejected at validation.

Workflow step types are `task`, `store.query`, `store.execute`, `http.request`, and `respond`. Task steps require successful task output or fail the workflow. Store parameters are bound as strings; use parameter placeholders (`?1` for SQLite and `$1` for PostgreSQL) rather than interpolating untrusted input into SQL. A store query returns `rows`; execute returns `rows_affected`. HTTP steps default to GET and 30000 ms unless their own `timeout_ms` or their transport's `timeout_ms` is set; non-2xx responses fail the step. A non-null HTTP body defaults its content type to `application/json` unless transport or step headers set one.

Set `continue_on_error = true` only when downstream behavior explicitly handles the inserted step output `{"ok":false,"error":"...","message":"..."}`. A `respond` step yields `{ "ok": true, "value": ... }`. With no explicit endpoint result body, the result is the value of the last configured `respond` step, or `{}`. `result.include_steps` defaults to `true`; when true (or when result body is null), the HTTP body is the workflow envelope `{ok, steps, result}`. An explicit result body with `include_steps = false` returns only the rendered result value.

## Security and request bounds

Configure endpoint controls; do not substitute a proxy for them. The enforcement order is: server body limit, endpoint body limit and metadata restrictions, trusted client-IP resolution, endpoint deadline, non-queuing concurrency permit, rate limit, idempotency-key parsing, failure-block check, API key, challenge task, x402 task, then idempotency claim and action. An idempotent replay still consumes a rate-limit slot.

- `restrictions.allowed_content_types` applies only to non-empty bodies and compares the media type without parameters. `required_headers` must be present. Endpoint `body_limit_bytes` can only make the server bound smaller.
- `max_concurrency` is local to a process and immediately rejects excess requests; it does not queue.
- `restrictions.timeout_ms` covers the permit, rate limit, guards, idempotency, and action. It must be 1 through 86400000 ms.
- Rate limiting requires a persistent configured SQLite or PostgreSQL store. It uses a per-endpoint SHA-256-hashed rendered key and a fixed window. Use shared PostgreSQL for multiple replicas. Allowed responses include `RateLimit` and `RateLimit-Policy`; quota exhaustion adds `Retry-After` and returns 429.
- API-key comparison is constant-time. The optional `scheme` compares `"<scheme> <value>"`. All configured guards must pass.
- Challenge and x402 guards run configured tasks and require successful JSON output. `success_path` selects a boolean path; absent it accepts `ok`, `allowed`, or `verified`, plus `paid` for x402. x402 failure returns 402 and x402 metadata headers. Artur does not validate challenge or payment protocols beyond that task result.
- Failure blocking records failed API-key/challenge/x402 checks only. It is in-memory, endpoint-scoped, clears after successful guards, and must not be relied upon across restarts or replicas.
- Idempotency is only valid for POST, PUT, PATCH, and DELETE. Without its configured header, the request executes normally. With it, Artur fingerprints endpoint, method, path, params, query, and raw body; a completed matching request replays status/body/headers, an in-flight request is 409, and a different fingerprint is 422. Keys are exactly one through 255 non-control ASCII bytes and may occur once.

`{{client.ip}}` is the TCP peer unless both `artur.server.client_ip.header` (`x-forwarded-for` or `forwarded`) and nonempty trusted proxy CIDRs are configured. Artur trusts that header only from a direct peer inside a trusted CIDR, walks the chain right-to-left to the first untrusted address, and rejects a missing or malformed selected header from a trusted peer with 400. Never trust public forwarding headers without this configuration.

## API and health behavior

Built-in `GET /healthz` always returns process liveness and config version; it does not test stores, child executables, transports, or readiness:

```json
{"ok":true,"version":1}
```

There is no separate built-in readiness endpoint. Configure a route only if its action safely performs the required readiness check.

Typical synchronous task response:

```json
{"ok":true,"task":"echo","status_code":0,"stdout":"hello\n","stderr":"","timed_out":false,"duration_ms":4}
```

Async poll response while running:

```json
{"id":"<UUID>","status":"running","task":"long_job","result":null}
```

Errors are JSON objects with `error` and `message`. Relevant status mappings are: 400 malformed request; 402 failed x402 guard; 403 failed API key or challenge; 404 unknown job; 409 in-flight idempotency key; 413 body limit; 415 rejected content type; 422 idempotency mismatch; 429 concurrency, failure-block, or rate-limit rejection; 502 task/store/outbound HTTP failure; and 504 endpoint timeout. Errors for 415, 429, and 504 use `application/problem+json`. Do not infer domain success from a 2xx task response; inspect `ok`, task output, workflow output, and downstream effects.

## Deployment, secrets, and recovery

Bind to loopback by default and use a trusted reverse proxy for TLS, network policy, and coarse filtering. Run as an unprivileged service user with narrowly scoped filesystem and network permissions. Preserve persistent SQLite storage on a durable volume for a single replica; use PostgreSQL for rate limits or idempotency shared by replicas. Do not use `:memory:` for either control.

Never pass secrets in task arguments because process listings can expose them. Use environment references and task environment injection, avoid logging rendered configuration or request headers, and rotate exposed credentials outside Artur. Ensure the proxy CIDRs precisely enumerate direct proxies before enabling forwarded-client resolution.

On shutdown, Artur drains requests after Ctrl-C or SIGTERM. If `runtime.shutdown_timeout_secs` is set and draining exceeds it, the process returns an error. Investigate active child tasks, endpoint deadlines, and store contention before restarting. A restart discards async job status and failure blocks; do not claim recovery, retry, or durable job semantics that Artur does not provide.

## Troubleshooting and validation

Use validation before every config deployment:

```bash
cargo run -- --config /protected/Config.toml check
```

For source changes, run exactly:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
docker build -t artur:local .
```

For a failed request, first identify its status category, then check the matching deterministic control: body and content metadata (400/413/415), proxy chain (400), guard task JSON and task timeout (402/403), idempotency fingerprint or active claim (409/422), local concurrency/rate store/failure block (429), endpoint timeout (504), or task/store/transport stderr and reachability (502). Do not increase timeouts, limits, or disable protections merely to make a test pass.

## Prohibited agent actions

- Do not fetch, start from, or modify an unreviewed remote config.
- Do not execute configured tasks or mutating endpoints solely to inspect behavior.
- Do not bypass or weaken API keys, challenge/x402 guards, body limits, content restrictions, idempotency, rate limits, concurrency, deadlines, trusted-proxy policy, or process isolation.
- Do not store secrets in TOML, source, commits, task arguments, test fixtures, or output.
- Do not represent `/healthz` as dependency readiness, async jobs as durable, or failure blocks/concurrency as distributed controls.
- Do not interpolate untrusted values into workflow SQL; bind them through `params`.
- Do not delete idempotency or rate-limit state to evade a live protection decision without explicit operator authorization and incident procedure.

## Final agent checklist

- [ ] Config is local, reviewed, version 1, and uses `[artur]` with at least one endpoint.
- [ ] Every referenced task, store, transport, dependency, executable, path, and environment variable is present and approved.
- [ ] All external effects and secret flows were assessed; no secret is placed in an argument or log.
- [ ] Bounds, deadlines, guards, idempotency, and client-IP trust match the route's risk and deployment topology.
- [ ] SQLite durability or shared PostgreSQL use matches replica count; no unsupported durable-job claim is made.
- [ ] `check` succeeded before start; `/healthz` and the smallest approved route check were verified.
- [ ] Source-change validation commands completed successfully, and recovery preserves the last validated configuration.
