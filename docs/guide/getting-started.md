# Getting started

Artur reads one TOML configuration supplied through `--config` or `ARTUR_CONFIG`.
Install a released binary with Cargo, then validate before serving:

```bash
cargo install artur
artur --config examples/config.toml check
artur --config examples/config.toml
```

`check` parses, expands required `${NAME}` environment references, and validates routes and references. It does not bind a listener, connect to a store, or run a task.

The included example listens on `127.0.0.1:46796`. In a second terminal:

```bash
curl -sS http://127.0.0.1:46796/v1/hello
```

## Smallest server

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
body = { ok = true, service = "artur" }
```

Use a reverse proxy or TLS terminator when exposing Artur beyond a private network. Keep the configuration in a protected path: task commands and workflow URLs are trusted operator input.
