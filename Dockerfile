# syntax=docker/dockerfile:1
# check=skip=InvalidDefaultArgInFrom
# The toolchain to build viz-shell: Rust pinned in rust-toolchain.toml, just,
# the C linker, bats for the example tests, and vz built from this checkout,
# on the image vz passes as BASE: the library's vz-debian-trixie, with sudo,
# the docker CLI and locales.
#
# apt when Debian's version will do; otherwise the vendor's release, verified
# by sha256. Every tool is pinned to an exact version: the image's content hash
# then names one toolset, on every machine that builds it.

# The image this one builds on, passed by vz; `docker build .` needs
# --build-arg BASE.
ARG BASE

# Vendor releases, verified by sha256; bind-mounted below, so no layer keeps
# them. x86_64 only for now.
FROM scratch AS just
ADD --checksum=sha256:4a5cc2f53e6f0f8c59092a6cc38291eb729d46a7dd95d3ae582008881b84931d \
    https://github.com/casey/just/releases/download/1.58.0/just-1.58.0-x86_64-unknown-linux-musl.tar.gz \
    /just.tgz

# The checksum is the one published beside it, rustup-init.sha256.
FROM scratch AS rustup
ADD --chmod=755 --checksum=sha256:dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71 \
    https://static.rust-lang.org/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init \
    /rustup-init

FROM ${BASE} AS toolchain
ARG DEBIAN_FRONTEND=noninteractive
# git and curl; the linker rustc calls, and its C library; bats and its
# helper libraries, which the example tests load.
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl gcc git libc6-dev \
      bats bats-assert bats-support bats-file \
 && rm -rf /var/lib/apt/lists/*

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
