#!/usr/bin/env bats
# You are you inside: same user, ids, working directory and exit code. And
# the project without git: a folder with vz.yml; none, refused or allowed.

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

# Outside git: folders in the throwaway home, which lives in this repository;
# GIT_CEILING_DIRECTORIES=$HOME stops git, and vz's walk up for a configuration
# file, below the home.
# in_folder NAME ARGS...: vz, without the example's files, from ~/NAME.
in_folder() {
    local dir="$HOME/$1"
    shift
    mkdir -p "$dir" && cd "$dir"
    GIT_CEILING_DIRECTORIES=$HOME "$vz_bin" "$@"
}
# The banner shows above an interactive shell only: script gives vz a
# terminal, and the shell reads exit from it.
banner_in() {
    mkdir -p "$HOME/$1" && cd "$HOME/$1"
    echo exit | NO_COLOR=1 GIT_CEILING_DIRECTORIES=$HOME timeout 120 \
        script -qec "$(printf '%q ' "$vz_bin" new)" /dev/null
}
configured_folder() {
    mkdir -p "$HOME/pull-image-configured"
    printf 'extends: vz-debian-trixie\nbanner: true\n' > "$HOME/pull-image-configured/vz.yml"
}

# UNRUN: verify with just examples pull-image
@test "a folder with vz.yml and no git is a project" {
    configured_folder
    run --separate-stderr in_folder pull-image-configured -- sh -c 'echo "$VZ_REPO"'
    assert_success
    assert_output "$HOME/pull-image-configured"
}

# UNRUN: verify with just examples pull-image
@test "a folder with vz.yml and no git: no Branch:" {
    configured_folder
    run --separate-stderr banner_in pull-image-configured
    assert_success
    assert_output --partial "Repo: ~/pull-image-configured"
    refute_output --partial "Branch:"
}

# UNRUN: verify with just examples pull-image
@test "a folder without .git or vz.yml is refused" {
    run in_folder pull-image-loose -- true
    assert_failure
    assert_output --partial "not in a project: no .git or vz.yml here or above"
}

# UNRUN: verify with just examples pull-image
@test "require_project: false runs in the folder, with a warning" {
    printf 'name: anywhere\nextends: vz-debian-trixie\nrequire_project: false\n' \
        > "$HOME/.config/viz-shell/anywhere.vz.yml"
    run in_folder pull-image-loose -c anywhere -- sh -c 'echo "$VZ_REPO"'
    assert_success
    assert_line "$HOME/pull-image-loose"
    assert_line --partial "not in a project: running in $HOME/pull-image-loose"
}
