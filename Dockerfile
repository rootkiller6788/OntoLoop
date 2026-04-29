FROM rust:1.86 AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY autoloop-state-adapter ./autoloop-state-adapter
COPY state_store ./state_store

RUN cargo build --release --workspace --bin autoloop

FROM debian:bookworm-slim
WORKDIR /srv/autoloop

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/autoloop /usr/local/bin/autoloop
COPY deploy/config /srv/autoloop/config
COPY deploy/backup /srv/autoloop/backup

ENV RUST_LOG=info
ENV AUTOLOOP_CONFIG=/srv/autoloop/config/autoloop.prod.toml

EXPOSE 3000

CMD ["autoloop", "--config", "/srv/autoloop/config/autoloop.prod.toml"]

