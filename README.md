# artur

<img align="right" src="https://raw.githubusercontent.com/melonask/artur/refs/heads/main/logo.svg" alt="Artur logo" width="200" />

**A config-driven Rust HTTP gateway and package orchestrator.** Define routes in TOML to return JSON, invoke allowlisted processes, poll in-memory jobs, or compose task, SQL, HTTP, and response workflow steps. Artur deliberately has no built-in business domain.

[Documentation](https://melonask.github.io/artur/) · [Getting started](https://melonask.github.io/artur/guide/getting-started) · [Configuration](https://melonask.github.io/artur/guide/configuration) · [Repository](https://github.com/melonask/artur)

## At a glance

| Capability | Contract |
| --- | --- |
| Routes | `GET`, `POST`, `PUT`, `PATCH`, and `DELETE`; configured under `[[artur.endpoints]]`. |
| Actions | `respond.static`, `task.run`, `job.get`, and `workflow.run`. |
| Task execution | Child-process args, environment, working directory, stdin, timeout, output limits, JSON parsing, sync or in-memory async mode. |
| Workflows | Dependency DAG; each ready layer runs in parallel; steps can run tasks, bound SQL, HTTP requests, or return a value. |
| Controls | Body/content/header restrictions, deadlines, local concurrency, persistent rate limits and idempotency, API-key and task guards. |
| Built-in route | `GET /healthz` returns process liveness and config version, not dependency readiness. |

## Requirements and installation

The package declares Rust **1.97** (`rust-version`) and builds the `artur` binary. Repository e2e coverage also uses Node.js/npm and `npx`; Docker verification needs Docker.

```bash
cargo install artur
artur --config Config.toml check
artur --config Config.toml
```

The checked-in `Dockerfile` builds a runtime image with `artur`, Python 3, and `examples/`; its default command is `artur --config examples/service.toml`.

```bash
docker build -t artur:local .
docker run --rm -p 46796:46796 artur:local
curl --fail-with-body http://127.0.0.1:46796/healthz
```

## Quick start

Use the real checked-in [`examples/config.toml`](examples/config.toml), from the repository root so its relative script paths resolve.

```bash
cargo run -- --config examples/config.toml check
cargo run -- --config examples/config.toml
```

In another terminal:

```bash
curl --fail-with-body http://127.0.0.1:46796/healthz
curl --fail-with-body http://127.0.0.1:46796/v1/hello
curl --fail-with-body -X POST 'http://127.0.0.1:46796/v1/process/echo/alice?source=demo' \
  -H 'content-type: application/json' \
  -d '{"message":"hello"}'
```

That config also exposes an async task at `POST /v1/process/long-task` and its poll route at `GET /v1/jobs/{job_id}`.

## CLI reference

```text
artur --config <PATH-OR-HTTP(S)-URL> [check]
```

| Item | Behavior |
| --- | --- |
| `--config <location>` | Required unless `ARTUR_CONFIG` supplies it. Loads a local TOML path or an `http(s)` URL. |
| `ARTUR_CONFIG` | Environment fallback for `--config`. A command-line value takes precedence. |
| `check` | Loads, expands `${NAME}`, parses, and validates; does not bind, connect to stores, or run tasks. On success stdout is exactly `configuration valid`. |
| no subcommand | Starts the listener after loading and validating configuration. Logs are written through tracing (normally stderr). |
| exit status | `0` on successful check or clean server shutdown; nonzero when configuration, loading, binding, runtime, or shutdown fails. |
| `--help`, `--version` | Clap-provided standard help/version output. |

`RUST_LOG`, when parsable as a tracing filter, overrides `log.level`; otherwise the default is `artur=info,tower_http=info`. `log.format` accepts `json` or `pretty`; absent it uses tracing's compact formatter. Configuration URL loading occurs before `check` can validate it, so use reviewed local files for operational changes.

## Configuration model

The full commented schema is [`Config.example.toml`](Config.example.toml). `version = 1`, an `[artur]` table, and at least one endpoint are required. Artur ignores other package namespaces, enabling shared files; Artur definitions must remain under `[artur]`, not root-level `[server]`, `[[tasks]]`, or `[[endpoints]]`.

### Root and server parameters

| Group | Fields and defaults |
| --- | --- |
| Root | `version` must be `1`; `[log] level, format`; `[runtime] worker_threads, shutdown_timeout_secs, tmp_dir, max_payload_bytes`; `[paths.<id>] path, format`. The runtime metadata except `shutdown_timeout_secs` does not change request execution. |
| Stores | `[stores.<id>] driver` is `sqlite` or `postgres`, `url` is required, `connect_timeout_secs` is optional. SQLite accepts `sqlite://PATH`, `sqlite:PATH`, raw paths, or `:memory:`; `:memory:` is invalid for rate-limit and idempotency stores. For SQLite idempotency, `connect_timeout_secs` is its busy timeout (default 5 seconds), not a general store-connection timeout. |
| Transports | `[transports.http.<id>] base_url` is required; `headers` and `timeout_ms` are optional. |
| Server | `[artur.server] bind = "127.0.0.1"`, `port = 46796`, `body_limit_bytes = 1048576`; `[artur.server.client_ip] header` is `x-forwarded-for` or `forwarded`, with `trusted_proxy_cidrs`. |

`${NAME}` expansion happens before TOML parsing. Every reference must name an existing environment variable; malformed or unterminated references fail loading.

### Tasks and endpoints

| Definition | Fields |
| --- | --- |
| `[[artur.tasks]]` | Required `name`, `command`; `mode = "sync"` or `"async"`; `args`, `env`, templated `working_dir`; `inherit_env = true`; `success_exit_codes = [0]`; `timeout_ms = 30000`; `max_stdout_bytes = max_stderr_bytes = 1048576`; `stdout_format = "text"` or `"json"`. |
| Task stdin | `[artur.tasks.stdin] type = "none"` (default), `body`, `request_json`, or `template`; the latter requires `template`. |
| `[[artur.endpoints]]` | Required unique `name`, `method`, `path`, `action`; optional `task`, `response`, `body_limit_bytes`, `restrictions`, `security`, `idempotency`, `steps`, and `result`. Paths begin `/`; `{name}` and legacy `:name` parameters work. |
| Static response | `[artur.endpoints.response] status = 200`, JSON `body = {}`, and `headers`. Static bodies and headers are returned as configured, not template-rendered. |
| Restrictions | `[artur.endpoints.restrictions] allowed_content_types`, `required_headers`, `timeout_ms` (1–86400000), `max_concurrency` (1–1000000). An endpoint body limit can equal or reduce the server limit; it cannot increase it. |
| Workflow result | `[artur.endpoints.result] status = 200`, `body = null`, `headers`, `include_steps = true`; workflow steps have `id`, `type`, `depends_on = []`, and `continue_on_error = false`, plus type-specific fields. |

### Security and idempotency parameters

| Group | Fields and behavior |
| --- | --- |
| Rate limit | `[artur.endpoints.security.rate_limit] store, key, requests, window_secs`; persistent SQLite or PostgreSQL; fixed window and endpoint-scoped SHA-256 key hash. |
| API key | `[...api_key] value` required; `header = "authorization"`; optional `scheme` compares `"<scheme> <value>"` in constant time. |
| Task guards | `[...challenge]` or `[...x402] task`, optional `success_path`. Without it, challenge accepts boolean `ok`, `allowed`, or `verified`; x402 additionally accepts `paid`. |
| Failure block | `[...failure_block] key = "{{header.authorization}}"`, `max_failures = 5`, `window_secs = 300`, `block_secs = 900`; process-local and endpoint-scoped. |
| Idempotency | `[artur.endpoints.idempotency] store`, `header = "idempotency-key"`, `ttl_secs = 86400`, `max_response_bytes = 1048576`; only POST/PUT/PATCH/DELETE and never SQLite `:memory:`. |

## Operational and API usage

### Route actions, tasks, and jobs

| Action | Required configuration | HTTP result |
| --- | --- | --- |
| `respond.static` | `response` | Configured JSON body, status, and headers. |
| `task.run` | existing `task` | Sync: `TaskOutput`. Async: accepted job object immediately. |
| `job.get` | path contains `{job_id}` or `:job_id` | In-memory `JobRecord`, or `404` if absent or after restart. |
| `workflow.run` | one or more valid `steps` | Rendered workflow result; by default the envelope includes steps. |

Task `stdout_format = "json"` makes invalid stdout set `ok: false` with `json_parse_error`; a timed-out task has `timed_out: true`. Output byte limits truncate captured output and set `stdout_truncated`/`stderr_truncated`; they are not process-output kill limits. Async job states are `running`, `completed`, and `failed`; jobs are in memory and are lost on restart.

`stdin.type = "request_json"` writes a compact document with `method`, `client`, `uri`, `path`, `params`, `query`, `headers`, `body`, `body_json`, and `steps`. `body` and templates use lossy UTF-8 decoding for non-UTF-8 bytes.

### Template variables

Templates apply to task args/env/working directory/template stdin; rate/failure/API-key values; and workflow SQL, params, HTTP URL/headers/body, response values, and explicit result bodies. A whole JSON template retains its JSON type; embedded values stringify objects/arrays. Unknown keys render empty; an unclosed `{{` is an error.

| Variables | Meaning |
| --- | --- |
| `{{method}}`, `{{uri}}`, `{{path}}`, `{{client.ip}}`, `{{body}}` | Request basics. |
| `{{request}}` / `{{request_json}}`, `{{body_json}}` | Full context / parsed JSON body. |
| `{{param.name}}`, `{{query.name}}`, `{{header.name}}`, `{{env.NAME}}` | Path, query, lower-case header lookup, process environment. |
| `{{body_json.user.id}}`, `{{body_json.items.0}}` | JSON paths, including array indexes. |
| `{{steps}}` / `{{step}}` | Map of completed workflow step outputs. |
| `{{steps.id.path}}` / `{{step.id.path}}` | Completed prior workflow step output. |

### Workflows, stores, and HTTP transports

Ready dependency layers run concurrently; a step can see only completed earlier layers. Unknown dependencies and cycles fail validation. `continue_on_error = true` inserts `{"ok":false,"error":"...","message":"..."}`; use it only when downstream steps handle that shape.

| Step type | Required fields | Output contract |
| --- | --- | --- |
| `task` | `task` | Successful `TaskOutput`; a non-OK task fails the workflow. |
| `store.query` | `store`, `sql`, optional `params` | `{ok, store, operation:"query", rows}`. |
| `store.execute` | `store`, `sql`, optional `params` | `{ok, store, operation:"execute", rows_affected}`. |
| `http.request` | `transport` and/or `url`; optional `method`, `headers`, `body`, `timeout_ms` | `{ok,status,url,body,headers,json?,json_parse_error?}`; non-2xx fails. |
| `respond` | optional `value` | `{ok:true,value}`. |

Store parameters are strings and must use placeholders (`?1` SQLite, `$1` PostgreSQL), not interpolation. HTTP defaults to `GET` and 30000 ms; a step timeout overrides transport timeout. Absolute step URLs override a transport base URL. Transport headers are applied before step headers. A non-null HTTP body defaults to `application/json` unless either header set supplies content type.

With a null `result.body`, the result is the last configured `respond` value or `{}`. `result.include_steps` defaults true: null body or true returns `{"ok":true,"steps":{...},"result":...}`. An explicit body with `include_steps = false` returns only the rendered result. See [`examples/universal-composition.toml`](examples/universal-composition.toml) for a composed configuration.

### Protection order and client identity

Artur applies: server body limit; endpoint body/metadata checks; client-IP resolution; endpoint deadline; non-queuing concurrency; rate limit; idempotency header parsing; failure-block check; API key; challenge; x402; then idempotency claim/action. A replay still consumes rate quota. Required headers missing is `400`; a non-empty body with absent/disallowed media type is `415`.

`{{client.ip}}` is the TCP peer by default. Forwarding data is used only when a configured direct peer is within `trusted_proxy_cidrs`; Artur walks the chosen header right-to-left to the first untrusted address. A trusted peer with missing/malformed configured forwarding data receives `400`. Never trust public forwarding headers without this policy.

Idempotency keys are one to 255 non-control ASCII bytes and may occur only once. The fingerprint includes endpoint, method, path, params, query, and raw body. A completed matching request replays original status/body/headers; an active matching claim is `409`; a different fingerprint is `422`.

## Response examples and HTTP errors

```json
{"ok":true,"version":1}
```

```json
{"job_id":"4a15d7e2-0f30-45c0-8262-2cad1c939dd0","status":"running"}
```

```json
{"id":"4a15d7e2-0f30-45c0-8262-2cad1c939dd0","status":"running","task":"long_task","result":null}
```

```json
{"ok":true,"task":"echo_json","status_code":0,"stdout":"{\"message\":\"hello\"}\n","stderr":"","timed_out":false,"duration_ms":4,"json":{"message":"hello"}}
```

Artur-produced error bodies have `error` and `message`; x402 errors also include `x402_version` and `accepts` and send `x402-version: 1` plus `payment-required`. The server-wide body limiter runs before the endpoint handler, so its `413` response is not an Artur error-envelope contract.

| Status | Error / cause | Content type and headers |
| --- | --- | --- |
| `400` | Malformed request, required header, forwarding header, or idempotency key. | JSON. |
| `402` | x402 task guard rejected request. | JSON; `x402-version`, `payment-required`. |
| `403` | API key or challenge guard rejected request. | JSON. |
| `404` | Unknown async job. | JSON. |
| `409` | Matching idempotency request remains in progress. | JSON. |
| `413` | Server or endpoint body bound exceeded. | Endpoint-bound rejection is JSON; the server-wide pre-handler limiter response is not an Artur envelope contract. |
| `415` | Non-empty body has missing/disallowed content type. | `application/problem+json`. |
| `422` | Idempotency key reused with a different fingerprint. | JSON. |
| `429` | Endpoint concurrency, failure block, or rate limit. | `application/problem+json`; rate exhaustion adds `Retry-After`, `RateLimit`, `RateLimit-Policy`. |
| `502` | Process, store, or outbound HTTP failure. | JSON. |
| `504` | Endpoint deadline expired. | `application/problem+json`. |
| `500` | Configuration/internal/I/O failure during request processing. | JSON. |

## Library API

The crate exports `async load_config(location: &str) -> Result<AppConfig>` and `async build_router(config: AppConfig) -> Result<Router>`. `load_config` fetches local or HTTP(S) configuration, expands environment references, and validates. `build_router` validates again and creates the configured router, including `/healthz`; callers provide listener and process lifecycle management.

## Deployment, reliability, security, and troubleshooting

- Bind loopback by default; use a trusted reverse proxy for TLS, network policy, and coarse filtering. Artur controls remain route-specific defenses, not a substitute for network policy.
- Treat TOML as privileged code: it selects commands, SQL, env values, working directories, and outbound URLs. Run unprivileged with narrow filesystem/network access; prefer absolute executable paths.
- Do not place secrets in task args (process listings can expose them), logs, or commits. Use environment expansion/task env and deployment secret handling.
- Use durable storage for single-replica SQLite controls; use shared PostgreSQL for multi-replica rate limits/idempotency. Neither async jobs nor failure blocks are durable or distributed.
- On SIGTERM/Ctrl-C Artur drains requests. If `runtime.shutdown_timeout_secs` expires, shutdown returns an error; inspect active tasks, deadlines, and store contention rather than increasing limits blindly.

The checked-in [`docker-compose.yml`](docker-compose.yml) runs the local Artur image with [`examples/service.toml`](examples/service.toml), mounts it read-only at `/etc/melonask/Config.toml`, persists `/app/data` in the `artur-data` volume, and publishes `46796`. The separate [`examples/compose.yaml`](examples/compose.yaml) demonstrates one shared configuration mounted into Artur, Ladon, Pano, Oracles, and Bria.

```bash
docker compose up --build
```

| Symptom | Check and recovery |
| --- | --- |
| `check` fails | Set every `${NAME}`, fix schema/version/reference errors, then rerun `check`; do not start first. |
| `400` | Check required headers, idempotency-key syntax, and trusted-proxy forwarding data. |
| `415`/`413` | Send allowed media type for a non-empty body; reduce payload or set an intentional bounded limit. |
| `402`/`403` | Inspect approved guard-task JSON and credential delivery; do not bypass guards. |
| `409`/`422` | Poll/retry with the same request after completion, or use a new key only for a new logical operation. |
| `429` | Respect `Retry-After`; examine local concurrency, shared rate store, and failure-block failures. |
| `502`/`504` | Inspect task stderr, executable permissions, store/transport reachability, and intentional deadline. |

## Development and verification

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
docker build -t artur:local .
```

Validate the exhaustive checked-in schema without starting a listener:

```bash
cargo run -- --config Config.example.toml check
```

## License

MIT. See [LICENSE](LICENSE).
