#!/usr/bin/env bats
# Mounts show host paths inside, at the same path or at a target; the test
# makes the host side in the throwaway home, and runs vz from a repository
# inside the read-only ~/repos.

load ../helpers

setup_file() {
    example_setup_file
    mkdir -p "$HOME/repos/other" "$HOME/.config/gh" "$HOME/library"
    echo hello > "$HOME/repos/other/readme"
    echo shelf > "$HOME/library/book"
    echo "[user]" > "$HOME/.gitconfig"
}

skip_log() { VZ_LOG=viz_shell=debug inside true 2>&1 >/dev/null; }
# A shell command's error inside, on stdout.
inside_with_errors() { inside sh -c "{ $1; } 2>&1"; }

@test "the container starts" {
    run --separate-stderr inside true
    assert_success
}

@test "a mount on the repository is skipped, and says so" {
    run --separate-stderr skip_log
    assert_success
    assert_output --partial "skipping mount $app: it lands on the repository"
}

@test "a read-only mount is readable" {
    run --separate-stderr inside cat "$HOME/repos/other/readme"
    assert_success
    assert_output "hello"
}

@test "and not writable" {
    run --separate-stderr inside_with_errors "touch $HOME/repos/other/new"
    assert_failure
    assert_output --partial "Read-only file system"
}

@test "the repository inside it stays writable" {
    run --separate-stderr inside touch "$app/new"
    assert_success
}

@test "what you create in the repository is yours on the host" {
    inside touch "$app/new"
    run --separate-stderr stat -c %u "$app/new"
    assert_success
    assert_output "$(id -u)"
}

@test "a :rw mount is writable" {
    run --separate-stderr inside sh -c 'echo token > ~/.config/gh/hosts.yml'
    assert_success
}

@test "the host sees a change through a :rw mount" {
    inside sh -c 'echo token > ~/.config/gh/hosts.yml'
    run --separate-stderr cat "$HOME/.config/gh/hosts.yml"
    assert_success
    assert_output "token"
}

@test "a single file can be mounted" {
    run --separate-stderr inside cat "$HOME/.gitconfig"
    assert_success
    assert_output "[user]"
}

@test "a single file is read-only by default" {
    run --separate-stderr inside_with_errors 'echo x >> ~/.gitconfig'
    assert_failure
    assert_output --partial "Read-only file system"
}

@test "a mount with a target shows the host folder there" {
    run --separate-stderr inside cat "$HOME/.agents/library/book"
    assert_success
    assert_output "shelf"
}

@test "the folder around it is yours" {
    run --separate-stderr inside stat -c %u "$HOME/.agents"
    assert_success
    assert_output "$(id -u)"
}

@test "one source lands at several targets" {
    run --separate-stderr inside cat "$HOME/.config/agent/library/book"
    assert_success
    assert_output "shelf"
}

@test "the state folder around a mount stays writable" {
    run --separate-stderr inside touch "$HOME/.config/agent/notes"
    assert_success
}

@test "the mount point vz made in the state folder is yours" {
    inside true
    run --separate-stderr stat -c %u "$app/.vz_state$HOME/.config/agent/library"
    assert_success
    assert_output "$(id -u)"
}
