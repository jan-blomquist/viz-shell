# The default's image: the configurations that extend it start from it.
FROM debian:stable-slim
RUN mkdir -p /etc/stack && touch /etc/stack/base
