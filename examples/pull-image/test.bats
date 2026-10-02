#!/usr/bin/env bats
# You are you inside: same user, ids, working directory and exit code.

load ../helpers

@test "the container starts" {
    run --separate-stderr inside true
    assert_success
}

@test "you are you" {
    run --separate-stderr inside id -un
    assert_success
    assert_output "$(id -un)"
}

@test "with your uid and gid" {
    run --separate-stderr inside sh -c 'echo "$(id -u):$(id -g)"'
    assert_success
    assert_output "$(id -u):$(id -g)"
}

@test "your home is the host's home path" {
    run --separate-stderr inside sh -c 'echo ~'
    assert_success
    assert_output "$HOME"
}

@test "in the directory vz ran from" {
    run --separate-stderr inside pwd
    assert_success
    assert_output "$PWD"
}

@test "the repository is writable" {
    mkdir -p "$PWD/scratch"
    run --separate-stderr inside touch "$PWD/scratch/created"
    assert_success
}

@test "what you create is yours on the host" {
    mkdir -p "$PWD/scratch"
    inside touch "$PWD/scratch/created"
    run --separate-stderr stat -c %u "$PWD/scratch/created"
    assert_success
    assert_output "$(id -u)"
}

@test "the command's exit code passes through" {
    run --separate-stderr inside sh -c 'exit 3'
    assert_failure 3
}
