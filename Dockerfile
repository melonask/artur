# syntax=docker/dockerfile:1.7

FROM rust:1.97-bookworm AS builder
WORKDIR /usr/src/artur

COPY Cargo.toml ./
COPY src ./src

RUN cargo generate-lockfile && cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates python3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin artur

WORKDIR /app
COPY --from=builder /usr/src/artur/target/release/artur /usr/local/bin/artur
COPY examples ./examples
RUN mkdir -p /app/data && chown -R artur:artur /app

USER artur
EXPOSE 46796
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["python3", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:46796/healthz', timeout=2).read()"]

CMD ["artur", "--config", "examples/service.toml"]
