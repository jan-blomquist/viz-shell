# syntax=docker/dockerfile:1
# The development image for this repository: the Rust toolchain pinned in
# rust-toolchain.toml, fish, and vz built from this checkout, on BASE.
#
# apt when Debian's version will do; otherwise the vendor's release, verified
# by sha256. Every tool is pinned to an exact version: the image's content hash
# then names one toolset, on every machine that builds it.

# The image this one starts from. Under a global configuration whose image is
# the base, vz passes that image as BASE; alone, as in CI, Debian, pinned.
ARG BASE=debian:trixie-20260918-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a

# Static binaries, published by Docker as this image.
FROM docker:29.8.1-cli@sha256:018edbc908e08fcc9dbf029c812c34251e9b4719e6f71ca0e5eae2a987d014ca AS docker-cli

# Vendor releases, verified by sha256; bind-mounted below, so no layer keeps
# them. x86_64 only for now.
FROM scratch AS just
ADD --checksum=sha256:4a5cc2f53e6f0f8c59092a6cc38291eb729d46a7dd95d3ae582008881b84931d \
    https://github.com/casey/just/releases/download/1.58.0/just-1.58.0-x86_64-unknown-linux-musl.tar.gz \
    /just.tgz

# The checksum is the tarball's line in https://nodejs.org/dist/v24.21.0/SHASUMS256.txt.
FROM scratch AS node
ADD --checksum=sha256:fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6 \
    https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-x64.tar.xz \
    /node.tar.xz

# The checksum is the one published beside it, rustup-init.sha256.
FROM scratch AS rustup
ADD --chmod=755 --checksum=sha256:dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71 \
    https://static.rust-lang.org/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init \
    /rustup-init

FROM ${BASE} AS toolchain
ARG DEBIAN_FRONTEND=noninteractive
# What vz's features need, as the base Dockerfile installs it, so the image
# also stands alone: sudo, the docker CLI, en_US.UTF-8, CA certificates. On
# the base, these lines change nothing.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates locales sudo \
 && rm -rf /var/lib/apt/lists/* \
 && sed -i 's/^# *\(en_US.UTF-8\)/\1/' /etc/locale.gen \
 && locale-gen
ENV LANG=C.UTF-8
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/ /usr/local/libexec/docker/cli-plugins/

# fish, the shell viz-shell.yml names; git and curl; the linker rustc calls, and
# its C library.
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl fish gcc git libc6-dev xz-utils \
 && rm -rf /var/lib/apt/lists/*
# No greeting; a user's config still overrides it.
RUN printf 'set -g fish_greeting\n' > /etc/fish/conf.d/viz-shell.fish

RUN --mount=type=bind,from=just,source=/just.tgz,target=/tmp/just.tgz \
    tar -xzf /tmp/just.tgz -C /usr/local/bin --no-same-owner just

# Rust where the official image keeps it, writable by every user as there;
# crate downloads land in the registry folder, kept as state.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN --mount=type=bind,from=rustup,source=/rustup-init,target=/tmp/rustup-init \
    /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain 1.98.1 \
      -c rustfmt -c clippy -t x86_64-unknown-linux-musl \
 && chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"

# Development only, until personal image overlays land: Node as the official
# image lays it out, and the coding agent this repository is developed with.
RUN --mount=type=bind,from=node,source=/node.tar.xz,target=/tmp/node.tar.xz \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 --no-same-owner \
      --exclude=node-v24.21.0-linux-x64/CHANGELOG.md \
      --exclude=node-v24.21.0-linux-x64/LICENSE \
      --exclude=node-v24.21.0-linux-x64/README.md
ENV DISABLE_AUTOUPDATER=1 \
    npm_config_fund=false \
    npm_config_update_notifier=false
RUN npm install -g @anthropic-ai/claude-code@2.1.285 && npm cache clean --force

FROM toolchain AS vz
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
 && cp target/x86_64-unknown-linux-musl/release/viz-shell /usr/local/bin/viz-shell

FROM toolchain
COPY --from=vz /usr/local/bin/viz-shell /usr/local/bin/viz-shell
RUN ln -s viz-shell /usr/local/bin/vz
CMD ["sh", "-c", "rustc --version && cargo --version && docker --version && vz --version"]
