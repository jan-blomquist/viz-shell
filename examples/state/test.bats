#!/usr/bin/env bats
# State survives the container, kept in .vz_state at its container path.

load ../helpers

# Each test starts with no state: those that read it, write it first.
setup() {
    example_setup
    rm -rf "$example_dir/.vz_state"
}

# One container writes state: a folder's file, an absolute path's, and the file
# with init content, edited.
write_state() {
    inside sh -c '
        echo kept > ~/.local/share/opencode/probe
        echo kept > /opt/data/probe
        echo "{\"edited\":true}" > ~/.config/opencode/opencode.json'
}

@test "the container starts" {
    run --separate-stderr inside true
    assert_success
}

@test "a file starts with its init content" {
    run --separate-stderr inside cat "$HOME/.config/opencode/opencode.json"
    assert_success
    assert_output "{}"
}

@test "one container writes state" {
    run --separate-stderr write_state
    assert_success
}

@test "the next container reads what one wrote" {
    write_state
    run --separate-stderr inside cat "$HOME/.local/share/opencode/probe"
    assert_success
    assert_output "kept"
}

@test "absolute paths survive too" {
    write_state
    run --separate-stderr inside cat /opt/data/probe
    assert_success
    assert_output "kept"
}

@test "init is never written again" {
    write_state
    run --separate-stderr inside cat "$HOME/.config/opencode/opencode.json"
    assert_success
    assert_output '{"edited":true}'
}

@test "the host keeps it at its container path" {
    write_state
    run --separate-stderr cat "$example_dir/.vz_state$HOME/.local/share/opencode/probe"
    assert_success
    assert_output "kept"
}

@test "the host keeps an absolute path at its container path too" {
    write_state
    run --separate-stderr cat "$example_dir/.vz_state/opt/data/probe"
    assert_success
    assert_output "kept"
}

@test "folders above a state path belong to you" {
    run --separate-stderr inside sh -c 'stat -c %U ~/.local ~/.local/state | sort -u'
    assert_success
    assert_output "$(id -un)"
}
