# syntax=docker/dockerfile:1
# check=skip=InvalidDefaultArgInFrom
# The developer image: fish and the coding agents, on the toolchain image vz
# passes as BASE (Dockerfile, on the library's vz-debian-trixie). Every tool is
# pinned; downloads are verified by sha256.

# The image this one builds on, passed by vz; `docker build .` needs
# --build-arg BASE.
ARG BASE

# The checksum is the tarball's line in https://nodejs.org/dist/v24.21.0/SHASUMS256.txt.
FROM scratch AS node
ADD --checksum=sha256:fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6 \
    https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-x64.tar.xz \
    /node.tar.xz

FROM ${BASE}
ARG DEBIAN_FRONTEND=noninteractive
# fish, the shell dev.vz.yml names; xz to unpack Node.
RUN apt-get update \
 && apt-get install -y --no-install-recommends fish xz-utils \
 && rm -rf /var/lib/apt/lists/*
# No greeting; a user's config still overrides it.
RUN printf 'set -g fish_greeting\n' > /etc/fish/conf.d/vz.fish

# Node as the official image lays it out.
RUN --mount=type=bind,from=node,source=/node.tar.xz,target=/tmp/node.tar.xz \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 --no-same-owner \
      --exclude=node-v24.21.0-linux-x64/CHANGELOG.md \
      --exclude=node-v24.21.0-linux-x64/LICENSE \
      --exclude=node-v24.21.0-linux-x64/README.md
ENV DISABLE_AUTOUPDATER=1 \
    npm_config_fund=false \
    npm_config_update_notifier=false
RUN npm install -g opencode-ai@1.18.33 @openai/codex@0.159.2 @anthropic-ai/claude-code@2.1.285 \
 && npm cache clean --force
