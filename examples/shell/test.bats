#!/usr/bin/env bats
# shell: fish when no command is given; a warning and a fallback without it.

load ../helpers

# shell_runs SCRIPT [CONFIG]: the interactive shell, reading SCRIPT from stdin.
shell_runs() {
    echo "$1" | vz_example new ${2:+-c "$2"}
}
login_shell() { inside sh -c 'grep "^$(id -un):" /etc/passwd | cut -d: -f7'; }
missing_logs() { shell_runs 'true' missing 2>&1 >/dev/null; }

@test "the image builds and starts" {
    run --separate-stderr inside true
    assert_success
}

@test "no command: fish" {
    run --separate-stderr shell_runs 'status fish-path | xargs basename'
    assert_success
    assert_output "fish"
}

@test "SHELL names it, for commands too" {
    run --separate-stderr inside sh -c 'echo $SHELL'
    assert_success
    assert_output "/usr/bin/fish"
}

@test "and so does the user's passwd entry" {
    run --separate-stderr login_shell
    assert_success
    assert_output "/usr/bin/fish"
}

@test "fish's configuration is state" {
    run --separate-stderr shell_runs 'set -U greeting hello'
    assert_success
}

# Two containers: the first sets a universal variable, the second reads it.
@test "fish's universal variables outlive the container" {
    shell_runs 'set -U greeting hello'
    run --separate-stderr shell_runs 'echo $greeting'
    assert_success
    assert_output "hello"
}

@test "an absolute path, used as it is" {
    run --separate-stderr with_config absolute sh -c 'echo $SHELL'
    assert_success
    assert_output "/usr/bin/fish"
}

@test "missing from the image: sh instead" {
    run --separate-stderr shell_runs 'echo $SHELL' missing
    assert_success
    assert_output "/bin/sh"
}

@test "with a warning" {
    run --separate-stderr missing_logs
    assert_success
    assert_output --partial 'shell `zsh` is not in the image'
}
