# Example: building an application API with Artur

This document is a template README for a project that uses `artur` as a config-driven HTTP entrypoint.

Artur is intentionally generic. Your app decides what each endpoint means by connecting it to scripts, CLIs, or services in TOML.

## Install

```bash
cargo install artur
```

Or use the GitHub Packages container image from this repository:

```bash
docker run --rm -p 46796:46796 ghcr.io/melonask/artur:latest
```

## Run

```bash
artur --config http://example.com/config.toml
```

For local development:

```bash
artur --config examples/config.toml --port 46796
```

For Docker-based development, ensure the configuration binds to `0.0.0.0` inside the container so published ports are reachable from the host.

## Example endpoint map

```toml
version = 1

[artur.server]
bind = "127.0.0.1"
port = 46796

[[artur.endpoints]]
name = "create_challenge"
method = "POST"
path = "/v1/challenge"
action = "task.run"
task = "challenge_create"

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
timeout_ms = 10000
stdout_format = "json"
```

This example assumes there is a `challenge` executable in `PATH`. If your challenge implementation is a Rust binary, Python script, `npx` package, container wrapper, or service client, replace only the `command` and `args` in TOML.

## Example: create a space

```toml
[[artur.endpoints]]
name = "create_space"
method = "POST"
path = "/v1/space"
action = "task.run"
task = "space_create"

[[artur.tasks]]
name = "space_create"
mode = "sync"
command = "python3"
args = ["examples/scripts/space_create.py"]
timeout_ms = 30000
stdout_format = "json"

[artur.tasks.stdin]
type = "body"
```

The `space_create.py` script owns the application logic. It can verify a challenge, create a random `sid`, write to any database, allocate addresses, call blockchain tooling, start a background workflow, or return an ID for future polling.

## Example requests

Create a challenge:

```bash
curl -sS -X POST http://127.0.0.1:46796/v1/challenge
```

Create a space:

```bash
curl -sS http://127.0.0.1:46796/v1/space \
  -H 'content-type: application/json' \
  -d '{
    "challenge": {
      "parameters": {
        "algorithm": "PBKDF2/SHA-256",
        "cost": 5000,
        "expiresAt": 1767226200,
        "keyLength": 32,
        "keyPrefix": "00",
        "nonce": "9f7a4c1e2b8d43a6a7c9941e8f2d0b3c",
        "salt": "b42f7b0c18d44f12ab09e6a721bb2a91"
      },
      "signature": "7b6d7e0a9e89b1e6bbf0cb3f3a8a1d8dfb0e0dbb8120f8e4ecf2cf9d93a62b17"
    },
    "solution": {
      "counter": 8241,
      "derivedKey": "00f4a9cc3d93b0e45cf6c0d86d35e3f119c7bb77b07fb0e5fd7c51e4f0c733aa",
      "time": 418.73
    }
  }'
```

Read a space by path:

```bash
curl -sS http://127.0.0.1:46796/v1/space/012345678901234567890123
```

Read a space by header:

```bash
curl -sS http://127.0.0.1:46796/v1/space/ -H 'sid: 012345678901234567890123'
```

## Recommended architecture

Use Artur as the HTTP adapter and task orchestrator:

```text
HTTP request
  -> artur route from TOML
    -> configured task
      -> your domain code
        -> database / queue / service / chain tooling
      <- JSON result or job ID
  <- HTTP response
```

This keeps Artur simple and lets each project choose its own storage, task runner, security layer, and business logic.

## Production checklist

- Store config in a protected path or trusted HTTPS source.
- Do not let untrusted users edit Artur TOML.
- Prefer absolute executable paths.
- Use environment variables or secret managers instead of inline secrets.
- Set tight `timeout_ms` values.
- Put Artur behind TLS and authentication if it is reachable outside a private network.
- Use a durable queue or database for workflows that must survive process restarts.
- Build and test container images in CI before publishing them to GitHub Packages or another registry.
