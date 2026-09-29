#!/usr/bin/env bash
# The secure floor by default; sudo in the trusted profile.
source "$(dirname "$0")/../assert.sh"

capabilities() { inside awk '/^CapEff/ { print $2 }' /proc/self/status; }
no_new_privileges() { inside awk '/^NoNewPrivs/ { print $2 }' /proc/self/status; }

expect_success "the image builds and starts" inside true
expect_output "the floor: the shell holds no capabilities" "0000000000000000" capabilities
expect_output "and gains none through setuid programs" "1" no_new_privileges
expect_failure "so sudo does not work" inside sudo -n true

expect_output "trusted: sudo gives root" "0" with_profile trusted sudo -n id -u
expect_output "without a password prompt" "ok" with_profile trusted sudo -n sh -c 'echo ok'

no_sudo_logs() { VZ_LOG=warn with_profile no-sudo-image true 2>&1 >/dev/null; }
expect_contains "sudo granted to an image without it: a warning" "has no sudo" no_sudo_logs
