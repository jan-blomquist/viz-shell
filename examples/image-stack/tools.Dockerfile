# Stacks: `ARG BASE` makes vz build it on the image of the layers below,
# passed as BASE. With nothing below, the default applies.
ARG BASE=debian:stable-slim
FROM ${BASE}
RUN mkdir -p /etc/stack && touch /etc/stack/tools
