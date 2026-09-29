#!/usr/bin/env bash
# The Dockerfile bakes your user; the entrypoint accepts it as you.
source "$(dirname "$0")/../assert.sh"

expect_success "the image builds with your user and starts" inside true
expect_output "the baked user is you" "$(id -un)" inside id -un
expect_output "what the build installed in your home runs" \
    "hello from $(id -un), baked into the image" inside sh -c '"$HOME/.local/bin/hello"'
expect_output "your home belongs to you" "$(id -un)" inside stat -c %U "$HOME"
