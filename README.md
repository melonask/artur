# artur

<img align="right" src="https://raw.githubusercontent.com/melonask/artur/refs/heads/main/logo.svg" alt="Artur is a config-driven Rust HTTP server" width="200" />

`artur` is a config-driven Rust HTTP server.

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

Artur maps configured HTTP routes to generic actions:

| Action | Purpose |
| --- | --- |
| `respond.static` | Return configured JSON for health checks, metadata, mocks, or docs endpoints. |
| `process.run` | Run an allowlisted process synchronously or asynchronously. |
| `job.get` | Read the status and result of an async process started by `process.run`. |

A process can be a Python script, Rust CLI, shell script, `npx` command, binary in your repo, or any executable available to the service user.

## Minimal configuration

```toml
version = 1

[server]
bind = "127.0.0.1"
port = 46796
body_limit_bytes = 1048576

[[endpoints]]
name = "hello"
method = "GET"
path = "/v1/hello"
action = "respond.static"

[endpoints.response]
status = 200
body = { ok = true, service = "artur" }
```

Run it:

```bash
artur --config examples/config.toml
curl -sS http://127.0.0.1:46796/v1/hello
```

## Process endpoint

```toml
[[endpoints]]
name = "echo"
method = "POST"
path = "/v1/process/echo/{name}"
action = "process.run"
process = "echo_json"

[[processes]]
name = "echo_json"
mode = "sync"
command = "python3"
args = ["examples/scripts/echo.py", "--name", "{{param.name}}", "--source", "{{query.source}}"]
timeout_ms = 30000
stdout_format = "json"

[processes.stdin]
type = "request_json"
```

Call it:

```bash
curl -sS 'http://127.0.0.1:46796/v1/process/echo/alice?source=demo' \
  -H 'content-type: application/json' \
  -d '{"message":"hello"}'
```

The process receives a request JSON document on stdin:

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

## Async process endpoint

```toml
[[endpoints]]
name = "long_task"
method = "POST"
path = "/v1/process/long-task"
action = "process.run"
process = "long_task"

[[processes]]
name = "long_task"
mode = "async"
command = "python3"
args = ["examples/scripts/long_task.py"]
timeout_ms = 60000
stdout_format = "json"

[processes.stdin]
type = "body"

[[endpoints]]
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

Current jobs are stored in memory. Use an external process or service when you need durable queues, retries, distributed workers, or long-lived state.

## Template variables

Artur renders templates inside process args, env values, working directories, and `stdin.type = "template"` payloads.

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
| `{{env.NAME}}` | Environment variable from the Artur process. |
| `{{body_json.user.id}}` | Field inside a JSON request body. Array indexes are supported, such as `{{body_json.items.0}}`. |

Unknown template keys render as an empty string.

## Process stdin modes

```toml
[processes.stdin]
type = "none"
```

```toml
[processes.stdin]
type = "body"
```

```toml
[processes.stdin]
type = "request_json"
```

```toml
[processes.stdin]
type = "template"
template = "user={{body_json.user.id}}"
```

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
[[processes]]
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
- Keep `timeout_ms` small and specific per process.
- Avoid placing secrets in command-line args because they may be visible in process listings. Prefer env vars and short-lived credentials.
- Run Artur as an unprivileged service user.
- Use a reverse proxy for TLS, authentication, rate limiting, and request logging when exposing Artur outside localhost.
- Use async process mode for long tasks and external durable queues when results must survive restarts.

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

The e2e test suite starts the compiled `artur` binary and verifies configured static endpoints plus process endpoints backed by JavaScript (`node`), local `npx`, and a compiled Rust helper.

## Docker

The included `Dockerfile` builds a minimal runtime image with the `artur` binary and example configuration. The container uses `examples/docker.toml`, which binds to `0.0.0.0:46796` for Docker port publishing.

```bash
docker build -t artur:local .
docker run --rm -p 46796:46796 artur:local
```

GitHub Actions builds the image on pull requests and publishes it to GitHub Packages (`ghcr.io/melonask/artur`) on pushes to the default branch and on releases.

## License

- MIT license, in `LICENSE`
