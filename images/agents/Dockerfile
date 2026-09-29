# syntax=docker/dockerfile:1
# viz-shell-agents: viz-shell-base with coding agents and what they run on:
# Node for the agents, uv and Python for the scripts they write. The golden
# base a repository's own image starts FROM, adding its toolchain.
#
# Everything under /usr/local, as in the base. The agents keep their state in
# the home: declare it as `state` in the repository's configuration.

ARG BASE=ghcr.io/jan-blomquist/viz-shell-base:2026.9.1

FROM node:24.21.0-trixie-slim AS node
FROM ghcr.io/astral-sh/uv:0.12.20 AS uv

FROM ${BASE}
LABEL org.opencontainers.image.source="https://github.com/jan-blomquist/viz-shell" \
      org.opencontainers.image.description="viz-shell-agents: viz-shell-base with coding agents" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"

ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 python3-venv \
 && rm -rf /var/lib/apt/lists/*

# Node as the official image lays it out; npm's global prefix is /usr/local.
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx

COPY --from=uv /uv /uvx /usr/local/bin/

# No telemetry, no update or funding notices; no self-updates either, which
# could not write /usr/local as the user anyway: a new version is a new image.
ENV DO_NOT_TRACK=1 \
    DISABLE_AUTOUPDATER=1 \
    OPENSPEC_TELEMETRY=0 \
    PI_TELEMETRY=0 \
    npm_config_fund=false \
    npm_config_update_notifier=false

RUN npm install -g \
      opencode-ai@1.18.33 \
      @openai/codex@0.159.0 \
      @mariozechner/pi-coding-agent@0.73.1 \
      @fission-ai/openspec@1.13.2 \
      @anthropic-ai/claude-code@2.1.285 \
 && npm cache clean --force
