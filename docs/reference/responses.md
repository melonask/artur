# Responses

Artur returns JSON for successful static, task, workflow, and job responses. Error bodies contain `error` and `message`.

## Representative responses

The following are representative samples, not fixed response data.

```json
{"ok":true,"service":"artur"}
```

```json
{"error":"request","message":"missing required header authorization"}
```

## Status semantics

| Status | Meaning |
| --- | --- |
| `400` | Malformed request, invalid forwarding data, required header, or idempotency key. |
| `404` | A requested asynchronous job is absent. |
| `409` | Matching idempotency request is still processing. |
| `413` | Server or endpoint body limit was exceeded. |
| `415` | Non-empty request body has a missing or disallowed content type. |
| `422` | An idempotency key was previously used for a different request. |
| `429` | Concurrency, failure-block, or rate-limit rejection. |
| `504` | Endpoint deadline expired. |

Errors for `415`, `429`, and `504` use `application/problem+json`; the other listed errors use the normal JSON content type. Rate-limit responses include `Retry-After`, `RateLimit`, and `RateLimit-Policy`.
