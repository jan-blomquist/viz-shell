# syntax=docker/dockerfile:1
# The development image for this repository: the Rust toolchain pinned in
# rust-toolchain.toml, vz built from this checkout, and what `just examples`
# needs to run inside vz: the docker CLI, git, just, ssh for git over ssh, and
# sudo for the trusted profile; fish as the shell.

# Every tool is pinned to an exact version: the image's content hash then
# names one toolset, on every machine that builds it.
FROM docker:29.8.1-cli AS docker-cli

FROM rust:1.95.0-slim-trixie AS toolchain
RUN rustup component add rustfmt clippy \
 && rustup target add x86_64-unknown-linux-musl

# just, from its own musl release: Debian's lags behind; BuildKit checks the sha256.
FROM toolchain AS just
ADD --checksum=sha256:4a5cc2f53e6f0f8c59092a6cc38291eb729d46a7dd95d3ae582008881b84931d \
    https://github.com/casey/just/releases/download/1.58.0/just-1.58.0-x86_64-unknown-linux-musl.tar.gz \
    /tmp/just.tgz
RUN tar -xzf /tmp/just.tgz -C /usr/local/bin just

FROM toolchain AS vz
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
 && cp target/x86_64-unknown-linux-musl/release/viz-shell /usr/local/bin/viz-shell

FROM toolchain
RUN apt-get update \
 && apt-get install -y --no-install-recommends fish git openssh-client sudo \
 && rm -rf /var/lib/apt/lists/*
# Static binaries: the CLI talks to the host's daemon through the shared socket.
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/ /usr/local/libexec/docker/cli-plugins/
COPY --from=just /usr/local/bin/just /usr/local/bin/just
COPY --from=vz /usr/local/bin/viz-shell /usr/local/bin/viz-shell
RUN ln -s viz-shell /usr/local/bin/vz
CMD ["sh", "-c", "rustc --version && cargo --version && docker --version && vz --version"]
