#!/usr/bin/env bash
# The image is built from the Dockerfile next to vz.yml, once per content.
source "$(dirname "$0")/../assert.sh"

expect_success "the image builds and starts" inside true
expect_contains "tools installed by the Dockerfile run" "ripgrep" inside rg --version

startup_logs() { VZ_LOG=viz_shell=info inside true 2>&1 >/dev/null; }
expect_lacks "a second run does not build again" "building" startup_logs
