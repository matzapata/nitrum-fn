# Enclave image for `nitrum build` (always `docker build -f Dockerfile`).
# Data-plane is Alpine/musl, so the host is built for x86_64-unknown-linux-musl.
# Fargate API image: Dockerfile.api.
# DATA_PLANE_IMAGE comes from [runtime].data_plane in nitrum.toml.

ARG DATA_PLANE_IMAGE=ghcr.io/matzapata/nitrum/data-plane:latest
ARG RUST_IMAGE=rust:1.95-bookworm

FROM --platform=linux/amd64 ${RUST_IMAGE} AS builder

WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN --mount=type=cache,id=nitrum-fn-enclave-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=nitrum-fn-enclave-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=nitrum-fn-enclave-target,target=/build/target,sharing=locked \
    cargo build --locked --release --target x86_64-unknown-linux-musl -p host --bin nitrum-fn-host \
    && mkdir -p /out \
    && cp /build/target/x86_64-unknown-linux-musl/release/nitrum-fn-host /out/nitrum-fn-host

FROM --platform=linux/amd64 ${DATA_PLANE_IMAGE}

WORKDIR /app
COPY --from=builder /out/nitrum-fn-host /app/nitrum-fn-host
COPY nitrum.toml /app/nitrum.toml

EXPOSE 8080
ENV NITRUM_FN_PORT=8080

CMD ["/app/data-plane", "--config", "/app/nitrum.toml"]
