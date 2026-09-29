# Bakes your user into the image, so later steps can install into your home.
# The user must match you exactly: same name, uid and home; the entrypoint
# refuses any other.
FROM debian:stable-slim

ARG VZ_USER
ARG VZ_UID
ARG VZ_GID
ARG VZ_GROUP
ARG VZ_HOME
RUN groupadd -g "$VZ_GID" "$VZ_GROUP" \
 && useradd -u "$VZ_UID" -g "$VZ_GID" -d "$VZ_HOME" -m -s /bin/bash "$VZ_USER"

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# From here on, steps run as you and write into your home.
USER $VZ_USER
RUN mkdir -p "$VZ_HOME/.local/bin" \
 && printf '#!/bin/sh\necho "hello from %s, baked into the image"\n' "$VZ_USER" \
      > "$VZ_HOME/.local/bin/hello" \
 && chmod +x "$VZ_HOME/.local/bin/hello"
