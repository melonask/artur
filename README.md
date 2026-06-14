# artur

<img align="right" src="https://raw.githubusercontent.com/melonask/artur/refs/heads/main/logo.svg" alt="Artur is a config-driven Rust HTTP server" width="200" />

`artur` is a universal config-driven Rust HTTP gateway and package orchestrator.

It lets developers expose HTTP endpoints from TOML and attach each endpoint to a controlled action. The core server stays generic: it does not hardcode challenges, wallets, blockchains, databases, queues, or business workflows. Those belong in commands or scripts that `artur` starts from configuration.

```bash
cargo install artur
artur --config http://example.com/config.toml
```

Or run the container image published by GitHub Actions to GitHub Packages:

```bash
docker run --rm -p 46796:46796 ghcr.io/melonask/artur:latest
curl -sS http://127.0.0.1:46796/v1/hello
```

The default example listens on `127.0.0.1:46796`:

```bash
artur --config examples/config.toml
artur --config examples/config.toml --port 46796
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

[http]
bind = "127.0.0.1"
port = 46796
prefix = "v1"

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

Security guards are declared per endpoint. Guards run before the endpoint action and participate in failed-request blocking.

```toml
[artur.endpoints.security.failure_block]
key = "{{header.authorization}}"
max_failures = 5
window_secs = 300
block_secs = 900

[artur.endpoints.security.challenge]
task = "altcha_verify"
success_path = "verified"

[artur.endpoints.security.x402]
task = "x402_verify"
success_path = "paid"
```

Security tasks are normal Artur tasks. They should return a successful exit code and JSON with `ok`, `allowed`, `verified`, or `paid` set to `true`, or use `success_path` to name the exact boolean field.

## Challenge and space example

`examples/challenge-space.toml` shows how to model this kind of flow without hardcoding it into Artur:

- `POST /v1/challenge` runs an external `challenge create ...` command.
- `POST /v1/space` runs your application script, which can call `challenge verify ...`, allocate a random `sid`, assign resources, persist data, and return JSON.
- `GET /v1/space/{sid}` and `GET /v1/space/` delegate lookup to your application script.

This is only an example of how developers can work with Artur. ALTCHA-style challenges, wallets, blockchains, balances, deposits, and expenses are application concerns, not Artur core concepts.

```bash
export ARTUR_CHALLENGE_HMAC_SECRET='my-hmac-secret'
export ARTUR_CHALLENGE_HMAC_KEY_SECRET='my-key-secret'
export ARTUR_SPACE_DB='artur-example-space.sqlite3'
artur --config examples/challenge-space.toml
```

If your preferred challenge crate exposes a binary named `challenge`, this kind of TOML can call it directly:

```toml
[[artur.tasks]]
name = "challenge_create"
mode = "sync"
command = "challenge"
args = [
  "create",
  "--cost", "5000",
  "--random-counter",
  "--expires-in", "600",
  "--hmac-secret", "{{env.ARTUR_CHALLENGE_HMAC_SECRET}}",
  "--hmac-key-secret", "{{env.ARTUR_CHALLENGE_HMAC_KEY_SECRET}}",
]
stdout_format = "json"
```

The important part is that a developer can swap this for any other implementation by changing TOML, not recompiling Artur.

## Security and operations notes

- Treat the TOML file as privileged code. Anyone who can edit it can define which commands Artur runs.
- Prefer absolute command paths in production.
- Keep `timeout_ms` small and specific per task or HTTP step.
- Avoid placing secrets in command-line args because they may be visible in process listings. Prefer env vars and short-lived credentials.
- Run Artur as an unprivileged service user.
- Use a reverse proxy for TLS, authentication, rate limiting, and request logging when exposing Artur outside localhost.
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

The included `Dockerfile` builds a minimal runtime image with the `artur` binary and example configuration. The container uses `examples/docker.toml`, which binds to `0.0.0.0:46796` for Docker port publishing.

```bash
docker build -t artur:local .
docker run --rm -p 46796:46796 artur:local
```

GitHub Actions builds the image on pull requests and publishes it to GitHub Packages (`ghcr.io/melonask/artur`) on pushes to the default branch and on releases.

## License

- MIT license, in `LICENSE`
