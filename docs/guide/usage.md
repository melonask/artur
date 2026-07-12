# Usage

## Static and task responses

`respond.static` returns configured JSON. `task.run` starts one configured task. A synchronous task returns its captured output; an asynchronous task returns a job record and is read through a `job.get` route.

```toml
[[artur.endpoints]]
name = "echo"
method = "POST"
path = "/v1/echo/{name}"
action = "task.run"
task = "echo_json"

[artur.tasks.stdin]
type = "request_json"
```

```bash
curl -sS -X POST 'http://127.0.0.1:46796/v1/echo/alice?source=docs' \
  -H 'content-type: application/json' \
  -d '{"message":"hello"}'
```

## Idempotent writes

Use idempotency on `POST`, `PUT`, `PATCH`, or `DELETE` endpoints with a persistent SQLite or PostgreSQL store.

```toml
[artur.endpoints.idempotency]
store = "gateway"
header = "idempotency-key"
ttl_secs = 86400
max_response_bytes = 1048576
```

Send one non-empty, non-control ASCII header value up to 255 bytes. A completed request with the same fingerprint replays its original status, headers, and body; an in-progress request returns `409`, and a different request using the same key returns `422`.

## Client IP and rate limits

Forwarding headers are trusted only if the direct TCP peer belongs to a configured CIDR.

```toml
[artur.server.client_ip]
header = "x-forwarded-for"
trusted_proxy_cidrs = ["10.0.0.0/8"]

[artur.endpoints.security.rate_limit]
store = "gateway"
key = "{{client.ip}}"
requests = 60
window_secs = 60
```

For a multi-replica rate limit, use a shared PostgreSQL store.
