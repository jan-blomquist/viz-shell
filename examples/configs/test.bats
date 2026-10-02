#!/usr/bin/env bats
# Configurations: documents of one file or files of their own, each extending
# what it names; a configuration never extends itself.

load ../helpers

setup_file() {
    example_setup_file
    cp "$example_dir"/library/*.vz.yml "$HOME/.config/viz-shell/"
    mkdir -p "$HOME/repos/other"
    echo hello > "$HOME/repos/other/readme"
}

chain_line() { vz_example -c trusted --show-env | head -1; }
select_by_env() { VZ_CONFIG=isolated vz_example -- "$@"; }
# touch's error inside the container, on stdout.
touch_in() { "$@" sh -c "touch $HOME/repos/other/new 2>&1"; }
# One scratch container writes, the next reads: state outlives its container.
scratch_round_trip() {
    with_config scratch sh -c "echo kept > ~/scratch/probe" &&
        with_config scratch cat "$HOME/scratch/probe"
}
# stderr of an entry that vz refuses.
refused() { vz_example -c "$1" -- true 2>&1 >/dev/null; }

@test "the default mounts ~/repos" {
    run --separate-stderr inside cat "$HOME/repos/other/readme"
    assert_success
    assert_output "hello"
}

@test "read-only" {
    run --separate-stderr touch_in inside
    assert_failure
    assert_output --partial "Read-only file system"
}

@test "another configuration overrides the mode: writable" {
    run --separate-stderr with_config writable touch "$HOME/repos/other/new"
    assert_success
}

@test "false removes the mount: isolated" {
    run --separate-stderr with_config isolated test ! -e "$HOME/repos/other/readme"
    assert_success
}

# One test, two containers: what the first writes, the second reads.
@test "another document adds its own state, scratch, which outlives the container" {
    run --separate-stderr scratch_round_trip
    assert_success
    assert_output "kept"
}

@test "a file of its own extends default: its mounts and its image" {
    run --separate-stderr with_config debian12 \
        sh -c 'echo "$(cat ~/repos/other/readme) $(cut -d. -f1 /etc/debian_version)"'
    assert_success
    assert_output "hello 12"
}

@test "trusted.vz.yml's extends: trusted is the library's" {
    run --separate-stderr chain_line
    assert_success
    assert_output --partial "vz-debian-trixie (library) → trusted (library) → trusted:"
}

@test "both apply" {
    run --separate-stderr with_config trusted sh -c 'echo "$TRUSTED $WHO"'
    assert_success
    assert_output "library repo-trusted"
}

@test "VZ_CONFIG selects a configuration too" {
    run --separate-stderr select_by_env test ! -e "$HOME/repos/other/readme"
    assert_success
}

@test "an unknown configuration is refused, naming it" {
    run --separate-stderr refused nope
    assert_failure
    assert_output --partial 'no configuration `nope`'
}
