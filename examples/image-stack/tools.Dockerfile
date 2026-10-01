# Stacks: `ARG BASE` makes vz build it on the image of the configurations before it
# in the chain, passed as BASE. With nothing before it, the default applies.
ARG BASE=debian:stable-slim
FROM ${BASE}
RUN mkdir -p /etc/stack && touch /etc/stack/tools
