---
layout: home
hero:
  name: Artur
  text: Config-driven HTTP gateway
  tagline: Define routes, controlled process execution, and workflows in TOML.
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started
    - theme: alt
      text: GitHub
      link: https://github.com/melonask/artur
features:
  - title: Explicit routes
    details: Map HTTP methods and paths to static responses, tasks, jobs, or workflows.
  - title: Controlled execution
    details: Define every executable, argument, environment value, timeout, and output bound in configuration.
  - title: Endpoint safeguards
    details: Apply metadata restrictions, client-IP resolution, rate limits, guards, concurrency limits, and idempotency.
---

## Start safely

Validate configuration before binding a port or starting a process:

```bash
artur --config Config.toml check
```

Then start Artur with the same reviewed file. See the [getting-started guide](/guide/getting-started).
