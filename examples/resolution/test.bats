#!/usr/bin/env bats
# What vz refuses while reading and resolving configurations, before anything
# runs: no image is pulled or built here. Every case's folder holds the files of
# one refusal; src/config/mod.rs
# (load__each_resolution_example__refused_with_its_message) loads each of them
# and checks its message. Here, one refusal goes through the binary end to end.
# The library without a default: examples/library.

load ../helpers

# refusal CASE [ARGS...]: vz's error for the case's configuration files.
refusal() {
    local case=$1 file files=()
    shift
    for file in "$example_dir/$case"/*.yml; do
        files+=(-f "$file")
    done
    "$vz_bin" "${files[@]}" "$@" --show-effective-config 2>&1 >/dev/null
}

@test "an unknown name is refused, naming who extends it and it" {
    run --separate-stderr refusal unknown -c ci
    assert_failure
    assert_output --partial 'configuration `ci` extends `nope`'
    assert_output --partial 'no configuration `nope`'
}

@test "an unknown name is refused, listing the folders scanned" {
    run --separate-stderr refusal unknown -c ci
    assert_failure
    assert_output --partial \
        "scanned $HOME/.config/viz-shell, $app, $example_dir/unknown/ci.vz.yml"
}
