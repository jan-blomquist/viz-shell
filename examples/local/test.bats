#!/usr/bin/env bats
# A personal configuration: one of the repository's, named, extending its
# default, with its own Dockerfile stacked on the default's image.

load ../helpers

setup_file() {
    example_setup_file
    mkdir -p "$HOME/shared" "$HOME/mine"
    echo mine > "$HOME/mine/note"
}

show_config() { vz_example -c sally --show-effective-config; }
# touch's error inside the default's container, on stdout.
touch_shared() { inside sh -c "touch $HOME/shared/new 2>&1"; }
# The banner shows above an interactive shell only: script gives vz a
# terminal, and the shell reads exit from it.
banner_of() {
    echo exit | NO_COLOR=1 timeout 120 \
        script -qec "$(printf '%q ' "$vz_bin" "${configs[@]}" -c "$1" new)" /dev/null
}

@test "the default is the repository's" {
    run --separate-stderr inside sh -c 'echo "$WHO"'
    assert_success
    assert_output "repo"
}

@test "with ~/shared read-only" {
    run --separate-stderr touch_shared
    assert_failure
    assert_output --partial "Read-only file system"
}

@test "-c sally: her value over the default's" {
    run --separate-stderr with_config sally sh -c 'echo "$WHO"'
    assert_success
    assert_output "sally"
}

@test "her mode wins: ~/shared is writable" {
    run --separate-stderr with_config sally touch "$HOME/shared/new"
    assert_success
}

@test "her own mount" {
    run --separate-stderr with_config sally cat "$HOME/mine/note"
    assert_success
    assert_output "mine"
}

@test "her Dockerfile stacked on the default's image" {
    run --separate-stderr with_config sally test -e /etc/sally/marker
    assert_success
}

@test "the default's image has none of her Dockerfile" {
    run --separate-stderr inside test ! -e /etc/sally/marker
    assert_success
}

# The other --show-effective-config lines of this chain are a unit test on these
# files: src/config/mod.rs (load__each_example_s_layout__its_chain_and_images).
@test "her Dockerfile requires the default's image, and stacks on it" {
    run --separate-stderr show_config
    assert_success
    assert_output --partial \
        "# image: $example_dir/sally.Dockerfile (sally, $example_dir/sally.vz.yml, ARG BASE: required, stacks)"
}

# UNRUN: written in the audit pass, verify with just examples local
@test "banner: joe's chain, the default then his" {
    run --separate-stderr banner_of joe
    assert_success
    assert_output --partial "Chain: default → joe"
}

# UNRUN: written in the audit pass, verify with just examples local
@test "banner: the configuration, joe" {
    run --separate-stderr banner_of joe
    assert_success
    assert_output --partial "Config: joe"
}
