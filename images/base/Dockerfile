# syntax=docker/dockerfile:1
# viz-shell-base: a lean, general start for any repository's image. What a
# shell needs day to day, and what vz's own features use: sudo for
# `privileges.sudo`, the docker CLI for `share.docker`. No language
# toolchain, no user: vz adds you at start.
#
# Tools live under /usr/local, never in a home: the image serves every user,
# and `state` mounts in the home cannot shadow them.
#
# Every version is pinned: the image's own tag then names one toolset.

FROM docker:29.8.1-cli AS docker-cli

FROM debian:trixie-20260918-slim AS just
ADD --checksum=sha256:4a5cc2f53e6f0f8c59092a6cc38291eb729d46a7dd95d3ae582008881b84931d \
    https://github.com/casey/just/releases/download/1.58.0/just-1.58.0-x86_64-unknown-linux-musl.tar.gz \
    /tmp/just.tgz
RUN tar -xzf /tmp/just.tgz -C /usr/local/bin just

FROM debian:trixie-20260918-slim
LABEL org.opencontainers.image.source="https://github.com/jan-blomquist/viz-shell" \
      org.opencontainers.image.description="viz-shell-base: a lean start for vz images" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"

ARG DEBIAN_FRONTEND=noninteractive
# vz passes the host's LANG through; en_US.UTF-8, the common one, is generated.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash ca-certificates curl dnsutils fish git gnupg htop iproute2 iputils-ping jq less \
      locales lsof make nano netcat-openbsd openssh-client procps ripgrep sudo tmux tree \
      unzip wget xz-utils \
 && rm -rf /var/lib/apt/lists/* \
 && sed -i 's/^# *\(en_US.UTF-8\)/\1/' /etc/locale.gen \
 && locale-gen
ENV LANG=C.UTF-8

# Static binaries: the CLI reaches a shared daemon through its socket.
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/ /usr/local/libexec/docker/cli-plugins/
COPY --from=just /usr/local/bin/just /usr/local/bin/just

CMD ["bash"]
