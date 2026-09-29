# syntax=docker/dockerfile:1
# The development image for this repository: viz-shell-agents (images/), plus
# the Rust toolchain pinned in rust-toolchain.toml and vz built from this
# checkout. `just build-base-images` builds the base images first.

# Every tool is pinned to an exact version: the image's content hash then
# names one toolset, on every machine that builds it.
FROM rust:1.98.1-slim-trixie AS toolchain
RUN rustup component add rustfmt clippy \
 && rustup target add x86_64-unknown-linux-musl

FROM toolchain AS vz
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
 && cp target/x86_64-unknown-linux-musl/release/viz-shell /usr/local/bin/viz-shell

FROM ghcr.io/jan-blomquist/viz-shell-agents:2026.9.1
ARG DEBIAN_FRONTEND=noninteractive
# The linker rustc calls, and its C library.
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc libc6-dev \
 && rm -rf /var/lib/apt/lists/*
# Rust where the official image keeps it; crate downloads land in the registry
# folder, kept as state.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
COPY --from=toolchain /usr/local/rustup /usr/local/rustup
COPY --from=toolchain /usr/local/cargo /usr/local/cargo
COPY --from=vz /usr/local/bin/viz-shell /usr/local/bin/viz-shell
RUN ln -s viz-shell /usr/local/bin/vz
CMD ["sh", "-c", "rustc --version && cargo --version && docker --version && vz --version"]
