#!/usr/bin/env bats
# The host's network in another configuration; docker's own network by default.

load ../helpers

interfaces() { ls /sys/class/net | sort | tr '\n' ' '; }

setup_file() {
    example_setup_file
    # What a plain container on the host's network sees: the reference, also
    # right in a nested run, where the host is the daemon's.
    export host_interfaces=$(docker run --rm --network host "$stand_in_image" \
        sh -c "$(declare -f interfaces); interfaces")
}

inside_interfaces() { inside sh -c "$(declare -f interfaces); interfaces"; }
trusted_interfaces() { with_config trusted sh -c "$(declare -f interfaces); interfaces"; }
host_alias() { inside getent hosts host.docker.internal | awk '{ print $2 }'; }

@test "trusted: the host's network interfaces" {
    run --separate-stderr trusted_interfaces
    assert_success
    assert_output "$host_interfaces"
}

@test "default: docker's own network" {
    run --separate-stderr inside_interfaces
    assert_success
    assert_output "eth0 lo "
}

@test "where the host answers to host.docker.internal" {
    run --separate-stderr host_alias
    assert_success
    assert_output "host.docker.internal"
}
