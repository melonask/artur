---
name: artur
description: Operate, configure, test, and diagnose the Artur config-driven HTTP gateway without bypassing its route, process, workflow, store, or endpoint-security contracts.
---

# Artur agent operating guide

## Purpose and boundaries

Use this guide for reviewed Artur TOML, local operation, verification, and diagnosis. Artur reads a privileged config and maps configured routes to static JSON, allowlisted child processes, in-memory async jobs, or workflow DAGs.

**Facts:** Artur is not a TLS terminator, identity provider, sandbox, secret manager, durable queue, scheduler, retry system, or domain validator. Jobs and failure blocks are process-local and disappear on restart. `/healthz` is liveness only; it does not test stores, executables, or transports.

## Command-selection table

| Goal | Command | Side effects |
| --- | --- | --- |
| Validate reviewed config only | `artur --config /approved/Config.toml check` | Reads/fetches config, expands environment, parses and validates; no listener/store/task. |
| Run from source | `cargo run -- --config /approved/Config.toml` | Binds listener; requests can invoke configured effects. |
| Run installed binary | `artur --config /approved/Config.toml` | Same runtime effects. |
| Test liveness after approved start | `curl --fail-with-body http://127.0.0.1:46796/healthz` | Read-only HTTP request. |
| Verify source changes | Use the commands in [Exact command reference](#exact-command-reference). | Build/test/container work. |

## Prerequisites and features

| Requirement | Source-established detail |
| --- | --- |
| Rust | Cargo declares `rust-version = "1.97"`; edition 2024. |
| Config input | `--config <PATH-OR-HTTP(S)-URL>` is required unless `ARTUR_CONFIG` is set. Use a reviewed local path for operations. |
| Runtime access | The service user needs each configured executable, working directory, SQLite path/PostgreSQL target, and HTTP target. |
| Optional verification tools | Node.js/npm and `npx` are used by e2e tests; Docker builds the container. |
| Features | No Cargo feature flags are declared; `--all-features` remains the repository verification command. |

`RUST_LOG` overrides `log.level` only when it parses as a tracing filter. Default logging is `artur=info,tower_http=info`.

## Safe workflow

1. Read the target route plus referenced task, store, transport, templates, SQL, and guard tasks. Determine child-process, persistence, and outbound HTTP effects.
2. Obtain approval before starting a server or calling a mutating/effectful route. Do not execute a configured task merely to inspect it.
3. Keep config local, reviewed, access-controlled, and at `version = 1`. Supply every required `${NAME}` through the service environment.
4. Validate before start:
   ```bash
   artur --config /approved/Config.toml check
   ```
   Success writes `configuration valid` to stdout. A remote config URL is fetched before validation; do not use unreviewed URLs.
5. Start only after validation, check `/healthz`, then make the smallest approved functional request.
6. On failure, preserve sanitized output, restore the last validated config, correct the deterministic cause, rerun `check`, and then restart. Do not loosen bounds or guards to make a test pass.

## Exact command reference

```text
artur --config <PATH-OR-HTTP(S)-URL> [check]
```

| Invocation | stdout / exit behavior |
| --- | --- |
| `artur --config FILE check` | Prints exactly `configuration valid`, exits 0 on valid config; nonzero on load, expansion, parse, or validation failure. |
| `artur --config FILE` | Validates, binds configured `bind:port`, then serves until signal/error; clean shutdown exits 0. |
| `cargo run -- --config FILE check` | Source-tree equivalent of the installed check command. |
| `cargo run -- --config FILE` | Source-tree equivalent of serving. |
| `artur --help`, `artur --version` | Standard Clap help/version output. |

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
docker build -t artur:local .
cargo run -- --config Config.example.toml check
```

## Configuration editing model and resolution rules

### Structure and resolution

- Required: `version = 1`, `[artur]`, and one or more `[[artur.endpoints]]`.
- Shared root sections: `[log]`, `[runtime]`, `[paths.<id>]`, `[stores.<id>]`, `[transports.http.<id>]`. Other package namespaces are ignored.
- Artur-owned `server`, `tasks`, and `endpoints` belong under `[artur]`; root-level equivalents are not Artur settings.
- `${NAME}` expansion precedes TOML parsing. Names must start with a non-digit and contain only ASCII letters, digits, or `_`; unset, invalid, or unterminated references fail loading.
- Server defaults: `127.0.0.1:46796`, body limit `1048576`. Endpoint limits can equal or reduce, never increase, that body limit.
- HTTP step URL: rendered absolute `url` wins; otherwise Artur joins transport `base_url` and relative `url`. Transport headers are added before step headers.

### Editable parameters

| Area | Fields / defaults |
| --- | --- |
| Logging/runtime | `log.level`, `log.format = json|pretty`; runtime `worker_threads`, `shutdown_timeout_secs`, `tmp_dir`, `max_payload_bytes`. Only `shutdown_timeout_secs` affects Artur lifecycle. |
| Store | `driver = sqlite|postgres`, required `url`, optional `connect_timeout_secs`. SQLite URL may be `sqlite://PATH`, `sqlite:PATH`, raw path, or `:memory:`; never use `:memory:` for idempotency/rate limits. For SQLite idempotency, `connect_timeout_secs` is the busy timeout (default 5 seconds), not a general connection timeout. |
| Transport | Required `base_url`; optional `headers`, `timeout_ms`. |
| Server identity | `bind`, `port`, `body_limit_bytes`; client IP `header = x-forwarded-for|forwarded` plus nonempty `trusted_proxy_cidrs`. |
| Task | Required unique `name`, `command`; `mode = sync|async`, `args`, `env`, `working_dir`, `inherit_env=true`, `success_exit_codes=[0]`, `timeout_ms=30000`, output limits `1048576`, `stdout_format=text|json`, `stdin`. |
| Endpoint | Unique `name` and method/path pair; method `GET|POST|PUT|PATCH|DELETE`; path starts `/`; action, task/response/steps/result, body bound, restrictions, security, idempotency. Result fields are `status=200`, `body=null`, `headers`, `include_steps=true`; steps have `id`, `type`, `depends_on=[]`, `continue_on_error=false`, and type-specific fields. |

Use [`Config.example.toml`](Config.example.toml) as the complete commented schema. Paths accept `{name}` and legacy `:name`; duplicate normalized method/path pairs fail validation.

## Package contracts

### Actions and route results

| Action | Required input | Response shape |
| --- | --- | --- |
| `respond.static` | `[artur.endpoints.response]` | Configured JSON `body`, `status` (default 200), and headers. **Fact:** static values are not rendered as templates. |
| `task.run` | Existing `task` | Sync `TaskOutput`; async `{"job_id":"<UUID>","status":"running"}`. |
| `job.get` | Route path containing `{job_id}`/`:job_id` | `{"id":"<UUID>","status":"running|completed|failed","task":"...","result":TaskOutput|null}`. |
| `workflow.run` | One or more valid steps | Envelope `{"ok":true,"steps":{...},"result":...}` unless explicit result body uses `include_steps=false`. |

```toml
[[artur.endpoints]]
name = "echo"
method = "POST"
path = "/v1/echo/{name}"
action = "task.run"
task = "echo_json"

[[artur.tasks]]
name = "echo_json"
command = "python3"
args = ["examples/scripts/echo.py", "--name", "{{param.name}}"]
stdout_format = "json"

[artur.tasks.stdin]
type = "request_json"
```

```bash
curl --fail-with-body -X POST 'http://127.0.0.1:46796/v1/echo/alice?source=demo' \
  -H 'content-type: application/json' \
  -d '{"message":"hello"}'
```

### Task input and output

| `stdin.type` | Child stdin |
| --- | --- |
| `none` | No stdin pipe (default). |
| `body` | Lossy UTF-8 request body text. |
| `request_json` | Compact JSON request context. |
| `template` | Rendered required `template` string. |

`request_json` contains `method`, `client` as `{ "ip": "..." }`, `uri`, `path`, `params`, `query`, lower-cased `headers`, decoded `body`, parsed-or-null `body_json`, and completed `steps`.

| `TaskOutput` field | Meaning |
| --- | --- |
| `ok`, `task`, `status_code`, `stdout`, `stderr`, `timed_out`, `duration_ms` | Always serialized. |
| `json_parse_error`, `json` | Present when applicable to `stdout_format = "json"`. Invalid JSON makes `ok` false. |
| `stdout_truncated`, `stderr_truncated` | Present and true when captured output exceeds its configured byte bound. |

A timeout returns `ok:false`, null `status_code`, `timed_out:true`; non-success exit also yields `ok:false`. Capturing limits do not terminate a process for producing extra output.

### Templates

| Variables | Contract |
| --- | --- |
| `{{method}}`, `{{uri}}`, `{{path}}`, `{{client.ip}}`, `{{body}}` | Request values. |
| `{{request}}` / `{{request_json}}`, `{{body_json}}` | Complete request JSON / parsed body. |
| `{{param.NAME}}`, `{{query.NAME}}`, `{{header.name}}`, `{{env.NAME}}` | Path, query, lower-case header, process environment. |
| `{{body_json.user.id}}`, `{{body_json.items.0}}` | Object fields and array indexes. |
| `{{steps.ID.path}}`, `{{step.ID.path}}` | Output from completed earlier workflow steps. |

Templates render in task args/env/working directory/template stdin; security values; workflow SQL/params/HTTP URL/headers/body/respond value/result body. Whole-value JSON templates preserve type; embedded object/array values stringify. Unknown variables render empty; unclosed `{{` is a config error.

### Workflow, store, and transport contracts

| Step | Input | Successful output |
| --- | --- | --- |
| `task` | `task` | TaskOutput with `ok:true`; non-OK fails workflow. |
| `store.query` | `store`, `sql`, `params` | `{ok:true,store,operation:"query",rows:[...]}`. |
| `store.execute` | `store`, `sql`, `params` | `{ok:true,store,operation:"execute",rows_affected:N}`. |
| `http.request` | `transport` and/or `url`, optional method/headers/body/timeout | `{ok,status,url,body,headers,json?,json_parse_error?}`; any non-2xx fails. |
| `respond` | optional `value` | `{ok:true,value}`. |

Ready dependency layers execute in parallel and cannot read one another's output. Use `depends_on` for sequencing. With `continue_on_error = true`, the failed step instead becomes `{"ok":false,"error":"<code>","message":"<message>"}`.

**Caution:** bind SQL values through `params` (`?1` for SQLite, `$1` for PostgreSQL); do not template untrusted input into SQL. HTTP defaults are `GET` and 30000 ms; a non-null body defaults `content-type: application/json` unless transport/step headers set one.

### Security, idempotency, and health

| Control | Contract |
| --- | --- |
| Metadata | Required header missing: `400`. Non-empty body with absent/disallowed `allowed_content_types`: `415`. |
| Client IP | Direct TCP peer unless configured forwarding header comes from a direct peer in a trusted CIDR; trusted-peer missing/malformed header: `400`. |
| Concurrency | `max_concurrency` is process-local, non-queuing; excess is `429`. |
| Rate limit | Persistent SQLite/PostgreSQL fixed window; endpoint-scoped SHA-256 rendered key. Success adds `RateLimit`/`RateLimit-Policy`; exhaustion is `429` plus `Retry-After`. |
| Guards | API key is constant-time; all configured API/challenge/x402 guards pass. Challenge accepts `ok|allowed|verified`, x402 also `paid`, or configured boolean `success_path`. |
| Idempotency | POST/PUT/PATCH/DELETE only. Key is one 1–255-byte non-control ASCII header value. Same finished fingerprint replays status/body/headers; in flight `409`; mismatch `422`. |
| Health | `GET /healthz` returns `{"ok":true,"version":1}` and establishes process liveness only. |

Enforcement order: server body limit; endpoint body/metadata; client IP; deadline; concurrency; rate limit; idempotency parse; failure block; API key; challenge; x402; idempotency claim; action. A replay consumes rate quota. Failure blocks count guard failures only, clear after successful guards, and are not distributed.

## Error diagnosis and recovery

| Status / symptom | Cause to inspect | Safe recovery |
| --- | --- | --- |
| Config check failure | `${NAME}`, schema version, duplicate route/name, missing references, invalid bounds/CIDR. | Correct config and rerun `check`; do not start first. |
| `400` | Required header, idempotency syntax, forwarding header. | Correct request/proxy policy. |
| `402` / `403` | Payment/challenge task JSON or API credential. | Correct approved verifier/credential; do not bypass guard. |
| `404` job | Unknown ID or process restart. | Treat as unavailable; Artur cannot recover job state. |
| `409` / `422` | Active idempotency claim / differing fingerprint. | Wait and retry same logical request, or use a new key for a new operation. |
| `413` / `415` | Body bound / media type. | Reduce body or send allowed media type; retain intentional bounds. The endpoint `413` is an Artur JSON error; the server-wide pre-handler limiter is not an Artur error-envelope contract. |
| `429` | Local concurrency, failure block, rate quota. | Respect `Retry-After`; diagnose source without deleting state to evade protection. |
| `502` | Task spawn/exit, store, SQL, or HTTP transport failed. | Inspect sanitized stderr, permissions, SQL, reachability. |
| `504` | Endpoint deadline includes controls and action. | Diagnose latency/contention; do not blindly increase timeout. |

Artur-produced errors are JSON `{ "error": "<code>", "message": "..." }`. `415`, `429`, and `504` use `application/problem+json`. `402` includes x402 metadata and `x402-version: 1` / `payment-required` headers. The server-wide body limiter runs before Artur’s endpoint error mapping.

## Security and reliability guardrails

**Facts:** Artur drains after Ctrl-C/SIGTERM. If `runtime.shutdown_timeout_secs` expires while draining, it returns an error. SQLite may be persistent for a single replica; shared PostgreSQL is required for cross-replica rate/idempotency coordination.

**Cautions:** Treat config as executable control-plane input. Prefer absolute commands, unprivileged service users, minimal filesystem/network permissions, environment-based secrets, and a trusted TLS/network proxy. Avoid secrets in args, rendered config, headers, logs, fixtures, commits, and agent output.

**Prohibited actions:**

- Do not fetch, start from, or edit an unreviewed remote config.
- Do not invoke effectful/mutating routes or tasks without approval.
- Do not bypass or weaken body limits, content restrictions, client-IP policy, deadlines, concurrency, rate limits, guards, idempotency, or failure blocking.
- Do not interpolate untrusted values into workflow SQL.
- Do not claim `/healthz` is readiness or jobs/failure blocks/concurrency are durable or distributed.
- Do not delete idempotency/rate state to evade a live control without explicit operator authorization and procedure.

## Verification checklist

- [ ] Config is reviewed, local, version 1, and has `[artur]` plus endpoints.
- [ ] Every task, executable, directory, store, transport, dependency, and `${NAME}` is present and approved.
- [ ] Effects, SQL parameters, outbound calls, secret flow, body bounds, deadlines, and client-IP topology are assessed.
- [ ] Store choice matches replica topology; no durable-job or distributed failure-block claim is made.
- [ ] `artur --config … check` succeeded before start.
- [ ] After approved start, `/healthz` and the smallest approved route check passed.
- [ ] For source changes, formatting, check, clippy, tests, container build, and schema check completed.
