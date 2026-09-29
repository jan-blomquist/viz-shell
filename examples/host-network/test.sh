#!/usr/bin/env bash
# The host's network in a profile; docker's own network by default.
source "$(dirname "$0")/../assert.sh"

interfaces() { ls /sys/class/net | sort | tr '\n' ' '; }
# What a plain container on the host's network sees: the reference, also
# right in a nested run, where the host is the daemon's.
host_interfaces=$(docker run --rm --network host debian:stable-slim sh -c "$(declare -f interfaces); interfaces")
inside_interfaces() { inside sh -c "$(declare -f interfaces); interfaces"; }
trusted_interfaces() { with_profile trusted sh -c "$(declare -f interfaces); interfaces"; }

expect_output "trusted: the host's network interfaces" "$host_interfaces" trusted_interfaces
expect_output "default: docker's own network" "eth0 lo " inside_interfaces
