#!/usr/bin/env bats
# The secure floor by default; sudo in the trusted configuration.

load ../helpers

capabilities() { inside awk '/^CapEff/ { print $2 }' /proc/self/status; }
no_new_privileges() { inside awk '/^NoNewPrivs/ { print $2 }' /proc/self/status; }
# The container's own cgroup: its process limit.
pids_max() { "$@" cat /sys/fs/cgroup/pids.max; }
# sudo's error inside, on stdout.
sudo_on_the_floor() { inside sh -c 'sudo -n true 2>&1'; }
no_sudo_logs() { VZ_LOG=warn with_config no-sudo-image true 2>&1 >/dev/null; }

@test "the image builds and starts" {
    run --separate-stderr inside true
    assert_success
}

@test "the floor: the shell holds no capabilities" {
    run --separate-stderr capabilities
    assert_success
    assert_output "0000000000000000"
}

@test "and gains none through setuid programs" {
    run --separate-stderr no_new_privileges
    assert_success
    assert_output "1"
}

@test "and runs at most 512 processes" {
    run --separate-stderr pids_max inside
    assert_success
    assert_output "512"
}

@test "so sudo does not work" {
    run --separate-stderr sudo_on_the_floor
    assert_failure
    assert_output --partial '"no new privileges" flag is set'
}

@test "trusted: sudo gives root" {
    run --separate-stderr with_config trusted sudo -n id -u
    assert_success
    assert_output "0"
}

@test "without a password prompt" {
    run --separate-stderr with_config trusted sudo -n sh -c 'echo ok'
    assert_success
    assert_output "ok"
}

# Without the floor's limit, the host's default applies: max, or what the daemon sets.
@test "and not the floor's process limit" {
    run --separate-stderr pids_max with_config trusted
    assert_success
    refute_output "512"
}

@test "sudo granted to an image without it: a warning" {
    run --separate-stderr no_sudo_logs
    assert_success
    assert_output --partial "has no sudo"
}
