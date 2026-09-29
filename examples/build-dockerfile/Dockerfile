# A toolbox image. No user is created: vz adds yours when the container starts.
ARG DEBIAN_RELEASE=stable-slim
FROM debian:${DEBIAN_RELEASE}

RUN apt-get update \
 && apt-get install -y --no-install-recommends git curl ca-certificates ripgrep \
 && rm -rf /var/lib/apt/lists/*
