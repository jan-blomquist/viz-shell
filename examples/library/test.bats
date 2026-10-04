#!/usr/bin/env bats
# The library: configurations any repository extends by name, and runs by
# name; its base, or its own default, when the repository has none. These run
# from a second throwaway repository, ~/repos/bare; a test that wants the
# example's app.vz.yml in it copies it there.

load ../helpers

setup_file() {
    example_setup_file
    export library="$HOME/.config/viz-shell"
    cp "$example_dir"/library/*.vz.yml "$library/"
    printf 'EVERYWHERE=yes\n' > "$library/environment"
    printf 'TRUSTED_SECRET=s3cret\n' > "$library/environment.trusted"
    export repo="$HOME/repos/bare"
    git init -q "$repo"
}

# Each test starts in the bare repository, without app.vz.yml, and the library
# without a default.
setup() {
    example_setup
    cd "$repo"
    rm -f app.vz.yml "$library/default.vz.yml"
}

teardown_file() {
    (cd "$repo" && "$vz_bin" kill --all >/dev/null 2>&1) || true
    example_teardown_file
}

run_vz() { "$vz_bin" "$@"; }
with_repository_configs() { cp "$example_dir/app.vz.yml" "$repo/"; }
with_library_default() {
    printf 'extends: vz-debian-trixie\nenv: { defaults: { WHO: library-default } }\n' \
        > "$library/default.vz.yml"
}
# --show-effective-config's chain lines.
chain_lines() { run_vz --show-effective-config | grep -v '^# image:' | grep '^# '; }
# vz configs: name, file, what it extends.
config_rows() { run_vz configs | awk 'NR > 1 { print $1, $2, $3 }'; }

# UNRUN: verify with just examples library
@test "a repository without configuration runs the template default" {
    run --separate-stderr run_vz -- sh -c 'echo "$WHO $EVERYWHERE"'
    assert_success
    assert_output "library yes"
}

# UNRUN: verify with just examples library
@test "a library default runs by default" {
    with_library_default
    run --separate-stderr run_vz -- sh -c 'echo "$WHO"'
    assert_success
    assert_output "library-default"
}

# UNRUN: verify with just examples library
@test "a library default: the base, then it" {
    with_library_default
    run --separate-stderr chain_lines
    assert_success
    assert_output "# vz-debian-trixie (~/.config/viz-shell/vz-debian-trixie.vz.yml)
# default (~/.config/viz-shell/default.vz.yml)"
}

@test "-c runs a library configuration by name" {
    run --separate-stderr run_vz -c vz-debian-trixie -- sh -c 'echo "$WHO $EVERYWHERE"'
    assert_success
    assert_output "library yes"
}

@test "-c trusted: the base, then the library's trusted" {
    run --separate-stderr run_vz -c trusted -- sh -c 'echo "$WHO $TRUSTED_SECRET"'
    assert_success
    assert_output "library s3cret"
}

@test "the repository's default extends the library's base" {
    with_repository_configs
    run --separate-stderr run_vz -- sh -c 'echo "$WHO $EVERYWHERE"'
    assert_success
    assert_output "repo yes"
}

@test "the repository's default keeps the trusted secrets out" {
    with_repository_configs
    run --separate-stderr run_vz -- sh -c 'echo "$TRUSTED_SECRET"'
    assert_success
    assert_output ""
}

@test "the repository's trusted extends the library's" {
    with_repository_configs
    run --separate-stderr run_vz -c trusted -- sh -c 'echo "$WHO $TRUSTED_SECRET"'
    assert_success
    assert_output "repo-trusted s3cret"
}

@test "ci extends the repository's trusted" {
    with_repository_configs
    run --separate-stderr run_vz -c ci -- sh -c 'echo "$TRUSTED_SECRET"'
    assert_success
    assert_output "s3cret"
}

# Which rows vz configs lists, and in what order, is a unit test:
# src/config/configs.rs (list__both_scopes__the_repository_s_first_with_file_extends_and_changes).
@test "vz configs lists the library's trusted by its path" {
    with_repository_configs
    run --separate-stderr config_rows
    assert_success
    assert_output --partial "trusted ~/.config/viz-shell/trusted.vz.yml vz-debian-trixie"
}

@test "a filename means nothing: the base, renamed, is still it" {
    with_repository_configs
    mv "$library/vz-debian-trixie.vz.yml" "$library/mine.vz.yml"
    run --separate-stderr run_vz -- sh -c 'echo "$WHO $EVERYWHERE"'
    mv "$library/mine.vz.yml" "$library/vz-debian-trixie.vz.yml"
    assert_success
    assert_output "repo yes"
}

@test "the base, renamed: no template is written beside it" {
    with_repository_configs
    mv "$library/vz-debian-trixie.vz.yml" "$library/mine.vz.yml"
    run_vz -- true
    run --separate-stderr ls "$library"
    mv "$library/mine.vz.yml" "$library/vz-debian-trixie.vz.yml"
    assert_success
    refute_output --partial "vz-debian-trixie"
}
