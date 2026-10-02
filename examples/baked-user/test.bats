#!/usr/bin/env bats
# The Dockerfile bakes your user; the entrypoint accepts it as you.

load ../helpers

@test "the image builds with your user and starts" {
    run --separate-stderr inside true
    assert_success
}

@test "the baked user is you" {
    run --separate-stderr inside id -un
    assert_success
    assert_output "$(id -un)"
}

@test "what the build installed in your home runs" {
    run --separate-stderr inside sh -c '"$HOME/.local/bin/hello"'
    assert_success
    assert_output "hello from $(id -un), baked into the image"
}

@test "your home belongs to you" {
    run --separate-stderr inside stat -c %U "$HOME"
    assert_success
    assert_output "$(id -un)"
}
