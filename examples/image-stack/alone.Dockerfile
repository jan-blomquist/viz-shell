# Replaces: no `ARG BASE`, so it ignores the image of the configurations before it.
FROM debian:stable-slim
RUN mkdir -p /etc/stack && touch /etc/stack/alone
