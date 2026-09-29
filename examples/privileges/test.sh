#!/usr/bin/env bash
# The secure floor by default; sudo in the trusted profile.
source "$(dirname "$0")/../assert.sh"

capabilities() { inside awk '/^CapEff/ { print $2 }' /proc/self/status; }
no_new_privileges() { inside awk '/^NoNewPrivs/ { print $2 }' /proc/self/status; }
# The container's own cgroup: its process limit.
pids_max() { "$@" cat /sys/fs/cgroup/pids.max; }

expect_success "the image builds and starts" inside true
expect_output "the floor: the shell holds no capabilities" "0000000000000000" capabilities
expect_output "and gains none through setuid programs" "1" no_new_privileges
expect_output "and runs at most 512 processes" "512" pids_max inside
expect_failure "so sudo does not work" inside sudo -n true

expect_output "trusted: sudo gives root" "0" with_profile trusted sudo -n id -u
expect_output "without a password prompt" "ok" with_profile trusted sudo -n sh -c 'echo ok'
# Without the floor's limit, the host's default applies: max, or what the daemon sets.
floor_limit_lifted() { [ "$(pids_max with_profile trusted)" != 512 ]; }
expect_success "and not the floor's process limit" floor_limit_lifted

no_sudo_logs() { VZ_LOG=warn with_profile no-sudo-image true 2>&1 >/dev/null; }
expect_contains "sudo granted to an image without it: a warning" "has no sudo" no_sudo_logs
