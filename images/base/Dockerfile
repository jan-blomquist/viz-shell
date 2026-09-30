# syntax=docker/dockerfile:1
# viz-shell-base: what vz's features need to work, and nothing decided for
# you; a start for your own image. sudo for `privileges.sudo`, the docker CLI
# for `share.docker`, en_US.UTF-8 for the host's LANG, CA certificates. No
# tools, no language toolchain, no user: vz adds you at start.
#
# Tools live under /usr/local, never in a home: the image serves every user,
# and `state` mounts in the home cannot shadow them.
#
# apt when Debian's version will do; otherwise the vendor's release, verified
# by sha256. Every FROM is pinned by digest: the image's tag names one build.

# Static binaries, published by Docker as this image.
FROM docker:29.8.1-cli@sha256:018edbc908e08fcc9dbf029c812c34251e9b4719e6f71ca0e5eae2a987d014ca AS docker-cli

FROM debian:trixie-20260918-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
LABEL org.opencontainers.image.source="https://github.com/jan-blomquist/viz-shell" \
      org.opencontainers.image.description="viz-shell-base: what vz's features need, nothing else" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"

ARG DEBIAN_FRONTEND=noninteractive
# vz passes the host's LANG through; en_US.UTF-8, the common one, is generated.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates locales sudo \
 && rm -rf /var/lib/apt/lists/* \
 && sed -i 's/^# *\(en_US.UTF-8\)/\1/' /etc/locale.gen \
 && locale-gen
ENV LANG=C.UTF-8

# The CLI reaches a shared daemon through its socket.
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/ /usr/local/libexec/docker/cli-plugins/

CMD ["bash"]
