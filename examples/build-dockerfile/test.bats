#!/usr/bin/env bats
# The image is built from the Dockerfile next to its configuration, once per content.

load ../helpers

@test "the image builds and starts" {
    run --separate-stderr inside true
    assert_success
}

@test "tools installed by the Dockerfile run" {
    run --separate-stderr inside rg --version
    assert_success
    assert_output --partial "ripgrep"
}

startup_logs() { VZ_LOG=viz_shell=info inside true 2>&1 >/dev/null; }

@test "a second run does not build again" {
    inside true
    run --separate-stderr startup_logs
    assert_success
    refute_output --partial "building"
}
