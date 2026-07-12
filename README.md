# artur

<img align="right" src="https://raw.githubusercontent.com/melonask/artur/refs/heads/main/logo.svg" alt="Artur is a config-driven Rust HTTP server" width="200" />

`artur` is a universal config-driven Rust HTTP gateway and package orchestrator.

[Documentation](https://melonask.github.io/artur/) · [Getting started](https://melonask.github.io/artur/guide/getting-started) · [Configuration](https://melonask.github.io/artur/guide/configuration) · [Repository](https://github.com/melonask/artur)

It lets developers expose HTTP endpoints from TOML and attach each endpoint to a controlled action. The core server stays generic: it does not hardcode challenges, wallets, blockchains, databases, queues, or business workflows. Those belong in commands or scripts that `artur` starts from configuration.

```bash
cargo install artur
artur --config Config.toml
```

Or run the container image published by GitHub Actions to GitHub Packages:

```bash
docker run --rm -p 46796:46796 ghcr.io/melonask/artur:latest
curl -sS http://127.0.0.1:46796/v1/hello
```

The default example listens on `127.0.0.1:46796`:

```bash
artur --config examples/config.toml
```

## What Artur does

Artur maps configured HTTP routes to generic actions under the `[artur]` namespace:

| Action | Purpose |
| --- | --- |
| `respond.static` | Return configured JSON for health checks, metadata, mocks, or docs endpoints. |
| `task.run` | Run an allowlisted task synchronously or asynchronously. |
| `job.get` | Read the status and result of an async task started by `task.run`. |
| `workflow.run` | Run a DAG of task, store query, store execute, and response steps with dependency-based sequential or parallel execution. |

A task can be a Python script, Rust CLI, shell script, `npx` command, binary in your repo, or any executable available to the service user. For distributed deployments, workflow steps can also call already-running package services through shared `[transports.http.*]` profiles.

## Minimal configuration

```toml
version = 1

[artur.server]
bind = "127.0.0.1"
port = 46796
body_limit_bytes = 1048576

[[artur.endpoints]]
name = "hello"
method = "GET"
path = "/v1/hello"
action = "respond.static"

[artur.endpoints.response]
status = 200
body = { ok = true, service = "artur" }
```

Run it:

```bash
artur --config examples/config.toml
curl -sS http://127.0.0.1:46796/v1/hello
```

## Task endpoint

```toml
[[artur.endpoints]]
name = "echo"
method = "POST"
path = "/v1/process/echo/{name}"
action = "task.run"
task = "echo_json"

[[artur.tasks]]
name = "echo_json"
mode = "sync"
command = "python3"
args = ["examples/scripts/echo.py", "--name", "{{param.name}}", "--source", "{{query.source}}"]
timeout_ms = 30000
stdout_format = "json"

[artur.tasks.stdin]
type = "request_json"
```

Call it:

```bash
curl -sS 'http://127.0.0.1:46796/v1/process/echo/alice?source=demo' \
  -H 'content-type: application/json' \
  -d '{"message":"hello"}'
```

The task receives a request JSON document on stdin:

```json
{
  "method": "POST",
  "uri": "/v1/process/echo/alice?source=demo",
  "path": "/v1/process/echo/alice",
  "params": { "name": "alice" },
  "query": { "source": "demo" },
  "headers": { "content-type": "application/json" },
  "body": "{\"message\":\"hello\"}",
  "body_json": { "message": "hello" }
}
```

## Async task endpoint

```toml
[[artur.endpoints]]
name = "long_task"
method = "POST"
path = "/v1/process/long-task"
action = "task.run"
task = "long_task"

[[artur.tasks]]
name = "long_task"
mode = "async"
command = "python3"
args = ["examples/scripts/long_task.py"]
timeout_ms = 60000
stdout_format = "json"

[artur.tasks.stdin]
type = "body"

[[artur.endpoints]]
name = "get_job"
method = "GET"
path = "/v1/jobs/{job_id}"
action = "job.get"
```

Starting the task returns a job ID:

```json
{ "job_id": "4a15d7e2-0f30-45c0-8262-2cad1c939dd0", "status": "running" }
```

Then read it:

```bash
curl -sS http://127.0.0.1:46796/v1/jobs/4a15d7e2-0f30-45c0-8262-2cad1c939dd0
```

Current jobs are stored in memory. Use Bria or another durable service when you need durable queues, retries, distributed workers, or long-lived state.

## Template variables

Artur renders templates inside task args, env values, working directories, HTTP step URLs/headers/bodies, and `stdin.type = "template"` payloads.

| Template | Value |
| --- | --- |
| `{{method}}` | HTTP method. |
| `{{uri}}` | Full request URI. |
| `{{path}}` | Request path without query string. |
| `{{client.ip}}` | Resolved client IP address. By default this is the direct peer; it uses a forwarding header only when that peer is in a configured trusted-proxy CIDR. |
| `{{body}}` | Raw request body as text. |
| `{{request_json}}` or `{{request}}` | Full request context as compact JSON. |
| `{{param.name}}` | Path parameter, such as `{name}`. |
| `{{query.name}}` | Query string value. |
| `{{header.name}}` | Header value, lower-case lookup. |
| `{{env.NAME}}` | Environment variable from the Artur service process. |
| `{{body_json.user.id}}` | Field inside a JSON request body. Array indexes are supported, such as `{{body_json.items.0}}`. |

Unknown template keys render as an empty string.

## Task stdin modes

```toml
[artur.tasks.stdin]
type = "none"
```

```toml
[artur.tasks.stdin]
type = "body"
```

```toml
[artur.tasks.stdin]
type = "request_json"
```

```toml
[artur.tasks.stdin]
type = "template"
template = "user={{body_json.user.id}}"
```


## Universal shared configuration

Artur now accepts the same universal root sections used by `bria`, `ladon`, `oracles`, and `pano`. Shared profiles live at the root; Artur-owned runtime definitions live under `[artur]`. Other package namespaces are intentionally ignored by Artur so the same `Config.toml` can be mounted into all five services.

```toml
version = 1

[log]
level = "info"
format = "json"

[runtime]
max_payload_bytes = 1048576
shutdown_timeout_secs = 30

[stores.artur]
driver = "sqlite"
url = "sqlite://data/artur/api.sqlite3"

[stores.ladon]
driver = "sqlite"
url = "sqlite://data/ladon/addresses.db"

[stores.pano]
driver = "sqlite"
url = "sqlite://data/pano/events.db"

[stores.oracles]
driver = "sqlite"
url = "sqlite://data/oracles/rates.db"

[stores.bria]
driver = "sqlite"
url = "sqlite://data/bria/bria-state.db"

[ladon]
store = "ladon"

[pano]
store = "pano"

[oracles]
store = "oracles"

[bria.global.state]
backend = "sqlite"
store = "bria"
```

Artur intentionally does not accept root-level `[[endpoints]]`, `[[tasks]]`, or `[server]`. Package-owned configuration must live under `[artur]`, so the same file can also contain `[bria]`, `[ladon]`, `[oracles]`, and `[pano]` without ambiguity.

## Workflows and store operations

A `workflow.run` endpoint executes ready steps in parallel and waits for declared `depends_on` steps before continuing. Each step result is available to later steps through `{{steps.<id>...}}`, so task output can become SQL parameters or another task's input.

```toml
[[artur.endpoints]]
name = "create_space"
method = "POST"
path = "/v1/spaces"
action = "workflow.run"

[artur.endpoints.result]
include_steps = false
body = { ok = true, sid = "{{steps.sid.json.sid}}", prices_usd = "{{steps.oracles_prices.json.prices}}", bria_job = "{{steps.bria_paid_task.json.job_id}}" }

[[artur.endpoints.steps]]
id = "sid"
type = "task"
task = "sid_create"

[[artur.endpoints.steps]]
id = "insert"
type = "store.execute"
store = "artur"
depends_on = ["sid"]
sql = "INSERT INTO spaces (sid, payload) VALUES (?1, ?2)"
params = ["{{steps.sid.json.sid}}", "{{request_json}}"]

[[artur.endpoints.steps]]
id = "lookup"
type = "store.query"
store = "artur"
depends_on = ["insert"]
sql = "SELECT sid, payload FROM spaces WHERE sid = ?1"
params = ["{{steps.sid.json.sid}}"]
```

Use `type = "http.request"` when the other package is already running in another container or on another server:

```toml
[transports.http.ladon]
base_url = "http://ladon:4010/v1"
timeout_ms = 10000
headers = { authorization = "Bearer {{env.LADON_API_KEY}}" }

[[artur.endpoints.steps]]
id = "ladon_addresses"
type = "http.request"
transport = "ladon"
method = "POST"
url = "/addresses/checkout"
depends_on = ["sid"]
body = { sid = "{{steps.sid.json.sid}}", chains = ["evm", "solana", "btc"] }
```

HTTP step output is available as `{{steps.<id>.status}}`, `{{steps.<id>.body}}`, and `{{steps.<id>.json...}}`.

Use `examples/universal-composition.toml` for the full five-package demonstration: create a `sid`, retrieve addresses from `ladon`, track in `pano`, read token and coin USD prices from `oracles`, store the combined record, and launch paid work through `bria`.
`examples/compose.yaml` shows the same `Config.toml` mounted into `artur`, `ladon`, `pano`, `oracles`, and `bria` containers.

## Ready-to-use paid job service

The repository also includes `examples/service.toml` and `docker-compose.yml` for an end-to-end service that uses one shared config to create space ULIDs, top up per-chain token balances at a supplied current USD token rate, run immediate or async jobs, and return HTTP 402 x402-native payment requirements when the selected space balance is insufficient.

```bash
docker compose up --build
python3 tests/data_e2e.py
```

Clients can either top up the space first or submit an `x-payment` header for a specific job request.

## Endpoint security

Artur applies protections per endpoint; it is not a replacement for network policy,
TLS termination, or an upstream proxy. The server-wide `body_limit_bytes` is the
outer request-body limit. An endpoint can set a smaller `body_limit_bytes`; it
cannot increase the server-wide limit.

### Enforcement model

For a configured endpoint, Artur enforces protections in this order:

1. The server-wide body limit is applied while the request body is extracted.
2. The endpoint body limit, required headers, and allowed content types are checked.
3. Artur resolves `{{client.ip}}`, then starts the endpoint timeout (when configured).
4. The per-endpoint concurrency permit and rate limit are acquired.
5. If idempotency is enabled, its header is parsed; security guards then run in this order: existing failure block, API key, challenge task, and x402 task.
6. Artur claims an idempotency key, replays a completed matching response if present, or runs the endpoint action and stores its response.

Consequently, an idempotent replay still consumes rate-limit quota, and a failed
guard can contribute to a failure block. Failure blocks track guard failures only;
they are in-memory and local to an Artur process.

### Client IP and trusted proxies

Without `[artur.server.client_ip]`, `{{client.ip}}` is always the direct TCP peer.
Do **not** trust a public `Forwarded` or `X-Forwarded-For` header merely because it
is present. To use either header, set both the header mode and explicit CIDRs for
every proxy that can connect directly to Artur:

```toml
[artur.server.client_ip]
header = "x-forwarded-for"
trusted_proxy_cidrs = ["10.0.0.0/8", "2001:db8:1234::/48"]
```

```toml
[artur.server.client_ip]
header = "forwarded"
trusted_proxy_cidrs = ["192.0.2.0/24"]
```

`header` accepts only `"x-forwarded-for"` and `"forwarded"`. Artur consults the
selected header only when the direct peer belongs to a trusted CIDR. It parses the
chain right to left and selects the first address that is not trusted; if every
address in the chain is trusted, it uses the direct peer. A trusted direct peer
must supply a well-formed selected header, otherwise the request is rejected with
`400`.

### Request bounds and restrictions

Use a small endpoint limit as a second bound and restrict metadata that reaches an
action:

```toml
[[artur.endpoints]]
name = "create_upload"
method = "POST"
path = "/v1/uploads"
action = "respond.static"
body_limit_bytes = 16384

[artur.endpoints.restrictions]
allowed_content_types = ["application/json"]
required_headers = ["authorization", "idempotency-key"]
timeout_ms = 5000
max_concurrency = 32
```

`allowed_content_types` is checked only for a non-empty body and compares the media
type without parameters. A missing required header is `400`; a non-empty body with
an absent or disallowed content type is `415`; an endpoint body over its limit is
`413`. `max_concurrency` rejects excess concurrent requests with `429`, and
`timeout_ms` covers concurrency, rate limiting, guards, idempotency, and the action
and returns `504` when exceeded.

For uploads, send a small JSON document containing metadata and a presigned
object-store URL through Artur. Upload the file directly to the object store rather
than proxying large bytes through the gateway; this keeps the body limits bounded.

### Rate limits

Rate limits use a configured SQLite or PostgreSQL store and a fixed window:

```toml
[artur.endpoints.security.rate_limit]
store = "gateway"
key = "{{client.ip}}"
requests = 60
window_secs = 60
```

The key is a template, so common scopes include `{{client.ip}}`,
`{{header.authorization}}` for an API key, and `{{header.x-wallet}}` for a wallet.
Stored rate-limit keys are SHA-256 hashes scoped to the endpoint, rather than the
rendered key value. Use PostgreSQL when requests can reach multiple replicas so all
replicas share the counter; SQLite is appropriate for a single Artur instance with
a persistent local database.

An allowed response carries the current draft headers:

```text
RateLimit: "60";r=59;t=42
RateLimit-Policy: "60";q=60;w=60
```

When the quota is exhausted, Artur returns `429` with `Retry-After` (seconds),
`RateLimit`, and `RateLimit-Policy`.

### API keys, task guards, and failure blocks

An API key is compared in constant time. The default header is `authorization`; a
`scheme` prefixes the configured value with `"<scheme> "`:

```toml
[artur.endpoints.security.api_key]
header = "authorization"
scheme = "Bearer"
value = "${ARTUR_ADMIN_API_KEY}"
```

Challenge and x402 guards refer to normal, configured Artur tasks. A task guard
allows the request only when it exits successfully and returns JSON with the
configured boolean `success_path`; without `success_path`, challenge guards accept
`ok`, `allowed`, or `verified`, while x402 guards also accept `paid`.

```toml
[artur.endpoints.security.challenge]
task = "verify_altcha"
success_path = "verified"

[artur.endpoints.security.x402]
task = "verify_x402_payment"
success_path = "paid"
```

Place an ALTCHA verification task behind the rate limit, as Artur does by default.
The guard implementation must validate the server-side ALTCHA challenge data and
enforce replay protection; Artur only runs the task and evaluates its boolean policy
result. Likewise, x402 verification is delegated to its task; a failed x402 guard
returns `402`.

Failure blocking throttles repeated failed API-key, challenge, or x402 checks by a
template key:

```toml
[artur.endpoints.security.failure_block]
key = "{{client.ip}}"
max_failures = 5
window_secs = 300
block_secs = 900
```

The defaults are `key = "{{header.authorization}}"`, `max_failures = 5`,
`window_secs = 300`, and `block_secs = 900`. A currently blocked key receives
`429`; successful guards clear that endpoint/key's failure record.

### Idempotency

Idempotency stores completed responses and associates a key with the request
fingerprint. It supports SQLite and PostgreSQL stores:

```toml
[artur.endpoints.idempotency]
store = "gateway"
header = "idempotency-key"
ttl_secs = 86400
max_response_bytes = 1048576
```

The default header is `idempotency-key`; the default TTL is 86400 seconds and the
default maximum stored response is 1048576 bytes. The key must be one to 255
non-control ASCII bytes and may occur only once. A matching completed request
replays its original status, body, and headers. Reuse with a different fingerprint
returns `422`; a matching request still in progress returns `409`.

### Complete protected POST

This self-contained configuration uses a static action, so it does not depend on a
sample binary. It combines metadata restrictions, direct-peer rate limiting, an API
key, failure blocking, and idempotency. Security mechanisms configured together are
all required; Artur does not treat an API key and a challenge task as alternatives
within one endpoint.

```toml
version = 1

[stores.gateway]
driver = "sqlite"
url = "sqlite://data/artur/gateway.sqlite3"

[artur.server]
bind = "127.0.0.1"
port = 46796
body_limit_bytes = 1048576

[[artur.endpoints]]
name = "create_upload"
method = "POST"
path = "/v1/uploads"
action = "respond.static"
body_limit_bytes = 16384

[artur.endpoints.response]
status = 201
body = { accepted = true }

[artur.endpoints.restrictions]
allowed_content_types = ["application/json"]
required_headers = ["authorization", "idempotency-key"]
timeout_ms = 5000
max_concurrency = 32

[artur.endpoints.security.rate_limit]
store = "gateway"
key = "{{client.ip}}"
requests = 60
window_secs = 60

[artur.endpoints.security.api_key]
header = "authorization"
scheme = "Bearer"
value = "${ARTUR_UPLOAD_API_KEY}"

[artur.endpoints.security.failure_block]
key = "{{client.ip}}"
max_failures = 5
window_secs = 300
block_secs = 900

[artur.endpoints.idempotency]
store = "gateway"
header = "idempotency-key"
ttl_secs = 86400
max_response_bytes = 1048576
```

### Errors

Artur error bodies contain `error` and `message`. `400` covers malformed requests,
including invalid idempotency keys or forwarding headers; `409` is an in-flight
idempotency key; `413` is a body limit; `415` is a rejected content type; `422` is
an idempotency fingerprint mismatch; `429` is concurrency, failure-block, or rate
limit rejection; and `504` is an endpoint timeout. Error responses for `415`,
`429`, and `504` use `application/problem+json`; rate-limit responses also include
`Retry-After`, `RateLimit`, and `RateLimit-Policy`. Other listed errors retain the
normal JSON response content type.

## Configuration validation

Validate a universal `Config.toml` without binding a listener, connecting to a
store, or running a task:

```bash
artur --config Config.toml check
```

Artur expands `${NAME}` environment references before parsing configuration;
every referenced variable must be set.

## Security and operations notes

- Treat the TOML file as privileged code. Anyone who can edit it can define which commands Artur runs.
- Prefer absolute command paths in production.
- Keep `timeout_ms` small and specific per task or HTTP step.
- Avoid placing secrets in command-line args because they may be visible in process listings. Prefer env vars and short-lived credentials.
- Run Artur as an unprivileged service user.
- Use a reverse proxy for TLS, network-level authentication, coarse request filtering, and request logging when exposing Artur outside localhost. Keep Artur's endpoint-aware restrictions, rate limits, guards, and idempotency enabled for the routes they protect; proxy rate limiting is a coarse outer defense, not a substitute for them.
- Use async task mode for short-lived background work and Bria or another durable queue when results must survive restarts.

## Development

Prerequisites:

- Rust stable matching `rust-version` in `Cargo.toml`.
- Node.js/npm for the JavaScript and local `npx` e2e coverage.
- Docker for container build verification.

Run the same core checks as CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
docker build -t artur:local .
```

The e2e test suite starts the compiled `artur` binary and verifies configured static endpoints plus task endpoints backed by JavaScript (`node`), local `npx`, and a compiled Rust helper.

## Docker

The included `Dockerfile` builds a minimal runtime image with the `artur` binary and the paid-job service example. Its default command uses `examples/service.toml`; `docker-compose.yml` mounts that configuration and publishes port `46796`.

```bash
docker build -t artur:local .
docker run --rm -p 46796:46796 artur:local
```

GitHub Actions builds the image on pull requests and publishes it to GitHub Packages (`ghcr.io/melonask/artur`) on pushes to the default branch and on releases.

## License

- MIT license, in `LICENSE`
