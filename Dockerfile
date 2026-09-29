# syntax=docker/dockerfile:1
# The development image for this repository: the Rust toolchain pinned in
# rust-toolchain.toml, vz built from this checkout, and what `just examples`
# needs to run inside vz: the docker CLI, git, just, and ssh for git over ssh.

FROM docker:29-cli AS docker-cli

FROM rust:1.95.0-slim-trixie AS toolchain
RUN rustup component add rustfmt clippy \
 && rustup target add x86_64-unknown-linux-musl

FROM toolchain AS vz
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
 && cp target/x86_64-unknown-linux-musl/release/vz /usr/local/bin/vz

FROM toolchain
RUN apt-get update \
 && apt-get install -y --no-install-recommends git just openssh-client \
 && rm -rf /var/lib/apt/lists/*
# Static binaries: the CLI talks to the host's daemon through the shared socket.
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/ /usr/local/libexec/docker/cli-plugins/
COPY --from=vz /usr/local/bin/vz /usr/local/bin/vz
CMD ["sh", "-c", "rustc --version && cargo --version && docker --version && ls -l /usr/local/bin/vz"]
