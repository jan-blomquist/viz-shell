#!/usr/bin/env bats
# Image stacking along the chain: `ARG BASE` builds on the image before it;
# anything else replaces it; a configuration's image holds only what it extends.
# --show-effective-config's lines for each chain are unit tests on these files:
# src/config/mod.rs (load__each_example_s_layout__its_chain_and_images); here,
# one end to end.

load ../helpers

markers() { with_config "$1" sh -c 'echo $(ls /etc/stack)'; }
show_config() { vz_example -c "$1" --show-effective-config; }
images() { docker image ls --format '{{.Repository}}'; }
startup_logs() { VZ_LOG=viz_shell=info with_config tools true 2>&1 >/dev/null; }

@test "the default builds base.Dockerfile" {
    run --separate-stderr inside sh -c 'echo $(ls /etc/stack)'
    assert_success
    assert_output "base"
}

@test "a Dockerfile with ARG BASE stacks on it" {
    run --separate-stderr markers tools
    assert_success
    assert_output "base tools"
}

@test "one without ARG BASE replaces it" {
    run --separate-stderr markers alone
    assert_success
    assert_output "alone"
}

@test "a replacing Dockerfile on top of a stack leaves only itself" {
    run --separate-stderr markers deep
    assert_success
    assert_output "alone"
}

@test "a configuration that extends nothing, with a reference: the reference alone" {
    run --separate-stderr with_config pulled test ! -e /etc/stack
    assert_success
}

@test "a Dockerfile with ARG BASE stacks on a reference" {
    run --separate-stderr markers pulled-tools
    assert_success
    assert_output "tools"
}

@test "a configuration that extends nothing: no base below it" {
    run --separate-stderr markers bare-tools
    assert_success
    assert_output "tools"
}

@test "--show-effective-config says the top image stacks" {
    run --separate-stderr show_config tools
    assert_success
    assert_output --partial \
        "# image: $example_dir/tools.Dockerfile (tools, $example_dir/default.vz.yml, ARG BASE: stacks)"
}

@test "built images are named after the Dockerfiles' folder" {
    inside true
    run --separate-stderr images
    assert_success
    assert_output --partial "vz-image-stack"
}

@test "a second run builds nothing" {
    with_config tools true
    run --separate-stderr startup_logs
    assert_success
    refute_output --partial "building"
}
