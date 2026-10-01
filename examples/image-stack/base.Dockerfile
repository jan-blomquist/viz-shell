# The repository's image: every profile starts from it.
FROM debian:stable-slim
RUN mkdir -p /etc/stack && touch /etc/stack/base
