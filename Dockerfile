# syntax=docker/dockerfile:1
#
# Multi-stage build for ghost-node.
#   docker build -t ghost-node .
#   docker run --rm -p 30333:30333 -p 9933:9933 ghost-node --dev
#
# Mirrors the toolchain pinned in rust-toolchain.toml (rustc 1.88.0,
# wasm32-unknown-unknown target, rust-src component).

FROM rust:1.88-bookworm AS builder

# Substrate build-time dependencies (bindgen needs libclang; some crates
# shell out to protoc / pkg-config).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        clang \
        llvm \
        libclang-dev \
        protobuf-compiler \
        libudev-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Components required by rust-toolchain.toml / substrate-wasm-builder.
RUN rustup target add wasm32-unknown-unknown \
    && rustup component add rust-src

WORKDIR /ghost
COPY . .

# Real release build with embedded Wasm (no SKIP_WASM_BUILD). libclang and the
# gcc include paths are resolved dynamically so the build follows whatever
# versions the base image ships, matching the env documented in AGENTS.md.
RUN set -eux; \
    export LIBCLANG_PATH="$(dirname "$(find /usr/lib -name 'libclang.so' | head -n1)")"; \
    export BINDGEN_EXTRA_CLANG_ARGS="-I$(gcc -print-file-name=include) -I/usr/include/x86_64-linux-gnu -I/usr/include"; \
    export WASM_BUILD_WORKSPACE_HINT=/ghost; \
    cargo build --release --bin ghost-node; \
    cp target/release/ghost-node /usr/local/bin/ghost-node; \
    /usr/local/bin/ghost-node --version

FROM debian:bookworm-slim

# ca-certificates: outbound TLS (telemetry, bootnodes). curl: compose/RPC
# healthchecks. libudev1: shared-lib dependency of the node binary.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libudev1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 1000 -U -s /bin/sh -d /ghost ghost \
    && mkdir -p /data \
    && chown ghost:ghost /data

COPY --from=builder /usr/local/bin/ghost-node /usr/local/bin/ghost-node

USER ghost

# 30333 p2p, 9933 http/ws RPC, 9944 legacy ws RPC port (published per compose/ops)
EXPOSE 30333 9933 9944
VOLUME ["/data"]

ENTRYPOINT ["ghost-node"]
CMD ["--dev", "--base-path", "/data"]
