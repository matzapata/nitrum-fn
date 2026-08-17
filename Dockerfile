FROM rust:1.95-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates

RUN cargo build --release -p api --bin nitrum-fn-api

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/nitrum-fn-api /usr/local/bin/nitrum-fn-api

USER 65534:65534
EXPOSE 8080
ENV NITRUM_FN_PORT=8080

CMD ["nitrum-fn-api"]
