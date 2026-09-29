#!/usr/bin/env bash
# State survives the container, kept in .vz_state at its container path.
source "$(dirname "$0")/../assert.sh"

expect_success "the container starts" inside true
expect_output "a file starts with its init content" "{}" inside cat "$HOME/.claude.json"

expect_success "one container writes state" inside sh -c '
    echo kept > ~/.claude/probe
    echo kept > /opt/data/probe
    echo "{\"edited\":true}" > ~/.claude.json'
expect_output "the next container reads it" "kept" inside cat "$HOME/.claude/probe"
expect_output "absolute paths survive too" "kept" inside cat /opt/data/probe
expect_output "init is never written again" '{"edited":true}' inside cat "$HOME/.claude.json"

state="$example_dir/.vz_state"
expect_output "the host keeps it at its container path" "kept" cat "$state$HOME/.claude/probe"
expect_output "an absolute path, likewise" "kept" cat "$state/opt/data/probe"

expect_output "folders above a state path belong to you" "$(id -un)" \
    inside sh -c 'stat -c %U ~/.local ~/.local/state | sort -u'
