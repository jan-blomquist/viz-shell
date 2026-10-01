#!/usr/bin/env bats
# The environment's sources and their order, and that values stay out of sight.
# One case per source end to end; the precedence between them, files in order,
# globs and a configuration's changes are unit tests: src/env.rs
# (resolve__every_source__later_wins, glob_matches__a_name_the_pattern_covers__matches) and
# src/config/merge.rs (merge__env_extending__overrides_removes_and_appends).

load ../helpers

setup_file() {
    example_setup_file
    printf 'LEVEL="first file"\n' > "$HOME/example.env"
}

show() { inside sh -c "echo \"\$$1\""; }
show_env() { VZ_EXAMPLE_TOKEN=secret-value vz_example --show-env; }
logged_command() {
    VZ_EXAMPLE_TOKEN=secret-value VZ_LOG=viz_shell=debug vz_example -- true 2>&1 >/dev/null
}
# TERM comes from the host; one the image cannot describe falls back.
term_inside() { TERM=$1 inside sh -c 'echo $TERM'; }
# stderr of an entry that vz refuses.
refused_entry() { inside true 2>&1 >/dev/null; }

@test "a default" {
    run --separate-stderr show GREETING
    assert_success
    assert_output "hello"
}

@test "the repo placeholder is substituted with the repository root" {
    run --separate-stderr show REPO_ROOT
    assert_success
    assert_output "$PWD"
}

@test "a file beats a default" {
    run --separate-stderr show LEVEL
    assert_success
    assert_output "first file"
}

@test "passthrough beats files" {
    run --separate-stderr env LEVEL=host "$vz_bin" "${configs[@]}" -- sh -c 'echo "$LEVEL"'
    assert_success
    assert_output "host"
}

@test "--env beats everything" {
    run --separate-stderr env LEVEL=host "$vz_bin" "${configs[@]}" --env LEVEL=cli -- \
        sh -c 'echo "$LEVEL"'
    assert_success
    assert_output "cli"
}

@test "a configuration: null removes a default" {
    run --separate-stderr with_config ci sh -c 'echo "$GREETING"'
    assert_success
    assert_output ""
}

@test "--show-env names each source" {
    run --separate-stderr show_env
    assert_success
    assert_output --partial "env.files $HOME/example.env"
}

@test "and never a value" {
    run --separate-stderr show_env
    assert_success
    refute_output --partial "secret-value"
}

@test "docker gets the name" {
    run --separate-stderr logged_command
    assert_success
    assert_output --partial "--env VZ_EXAMPLE_TOKEN"
}

@test "never the value" {
    run --separate-stderr logged_command
    assert_success
    refute_output --partial "secret-value"
}

@test "a terminal the image does not know, a newer one's, falls back to xterm-256color" {
    run --separate-stderr term_inside xterm-ghostty
    assert_success
    assert_output "xterm-256color"
}

@test "a missing required: true file is refused, naming it" {
    mv "$HOME/example.env" "$HOME/example.env.away"
    run --separate-stderr refused_entry
    mv "$HOME/example.env.away" "$HOME/example.env"
    assert_failure
    assert_output --partial "example.env does not exist"
}
