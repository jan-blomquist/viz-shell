# sudo, for the trusted profile; the secure floor needs nothing of the image.
FROM debian:stable-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends sudo \
 && rm -rf /var/lib/apt/lists/*
