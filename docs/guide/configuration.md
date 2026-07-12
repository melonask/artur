# Configuration

The top-level `version` must be `1`, `[artur]` is required, and at least one `[[artur.endpoints]]` entry is required. Unknown fields inside Artur-owned endpoint structures are rejected.

## Server and logs

```toml
[log]
level = "artur=info"
format = "json" # "json" or "pretty"; omit for the compact default

[runtime]
shutdown_timeout_secs = 30

[artur.server]
bind = "127.0.0.1"
port = 46796
body_limit_bytes = 1048576
```

On `SIGINT` or `SIGTERM`, Artur stops accepting new work and drains active requests. `shutdown_timeout_secs` bounds that drain after the signal.

## Tasks and workflow HTTP transports

Tasks are named allowlisted executables. Templates can use request values such as `&#123;&#123;param.name}}`, `&#123;&#123;query.page}}`, `&#123;&#123;header.authorization}}`, `&#123;&#123;client.ip}}`, and `&#123;&#123;env.HOME}}`.

```toml
[[artur.tasks]]
name = "echo_json"
command = "python3"
args = ["examples/scripts/echo.py", "--name", "{{param.name}}"]
timeout_ms = 30000
stdout_format = "json"

[transports.http.inventory]
base_url = "https://inventory.internal/v1"
timeout_ms = 5000

[[artur.endpoints.steps]]
id = "inventory"
type = "http.request"
transport = "inventory"
url = "/items/{{param.name}}"
```

A transport supplies a base URL, default headers, and timeout. A relative `url` joins that base; an absolute `http://` or `https://` URL is sent as written and therefore bypasses the base URL.

## Stores

SQLite URLs use `sqlite://relative/path.db` or `sqlite:/absolute/path.db`. `connect_timeout_secs` is the SQLite busy timeout for idempotency records. PostgreSQL uses a normal connection URL.

```toml
[stores.gateway]
driver = "sqlite"
url = "sqlite://data/artur/gateway.sqlite3"
connect_timeout_secs = 10
```

See the repository's [`Config.example.toml`](https://github.com/melonask/artur/blob/main/Config.example.toml) for the complete schema.
