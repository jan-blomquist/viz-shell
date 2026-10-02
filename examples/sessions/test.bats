#!/usr/bin/env bats
# Named containers; persistent ones, attached to by index, name, or `attach: true`.
# Each test starts with no containers of this repository: it makes those it
# needs, so the names it expects are vz-<index>-sessions from index 0.

load ../helpers

setup() {
    example_setup
    vz_example kill --all >/dev/null 2>&1 || true
}

# vz ls without its header, in single spaces: name, configuration, persistent, state.
ls_rows() { vz_example ls | awk 'NR > 1 { print $1, $2, $3, $4 }'; }
# stderr of a vz that refuses.
attach_error() { vz_example attach 0 -- true 2>&1 >/dev/null; }
new_error() { vz_example -c kept new "$1" -- true 2>&1 >/dev/null; }
# A persistent container of `kept`, index 0, left running.
kept_container() { vz_example -c kept -- true; }

@test "vz ls without containers says so" {
    run --separate-stderr vz_example ls
    assert_success
    assert_output "No containers of this repository."
}

@test "a container is named, its hostname too" {
    run --separate-stderr inside cat /etc/hostname
    assert_success
    assert_output "vz-0-sessions"
}

@test "an ephemeral container is removed on exit" {
    inside true
    run --separate-stderr vz_example ls
    assert_success
    assert_output "No containers of this repository."
}

@test "the shell knows its container" {
    run --separate-stderr inside sh -c 'echo "$VZ_CONTAINER"'
    assert_success
    assert_output "vz-0-sessions"
}

@test "the shell knows its configuration, when one" {
    run --separate-stderr vz_example -c kept -- sh -c 'echo "$VZ_CONTAINER_CONFIG"'
    assert_success
    assert_output "kept"
}

@test "persistent: the container outlives its command, vz ls lists it running" {
    kept_container
    run --separate-stderr ls_rows
    assert_success
    assert_output "vz-0-sessions kept yes running"
}

@test "the next container takes the next free index" {
    kept_container
    run --separate-stderr inside cat /etc/hostname
    assert_success
    assert_output "vz-1-sessions"
}

@test "attaching across configurations is refused, naming the command to use" {
    kept_container
    run --separate-stderr attach_error
    assert_failure
    assert_output --partial "vz -c kept attach 0"
}

@test "attach by index, as you" {
    kept_container
    run --separate-stderr vz_example -c kept attach 0 -- \
        sh -c 'echo "$(id -un)@$(cat /etc/hostname)"'
    assert_success
    assert_output "$(id -un)@vz-0-sessions"
}

@test "a file written in the container is there on the next attach" {
    kept_container
    vz_example -c kept attach 0 -- touch /tmp/mark
    run --separate-stderr vz_example -c kept attach 0 -- test -e /tmp/mark
    assert_success
}

@test "attach alone: the only running container" {
    kept_container
    run --separate-stderr vz_example -c kept attach -- cat /etc/hostname
    assert_success
    assert_output "vz-0-sessions"
}

@test "a stopped one is started on attach" {
    kept_container
    docker stop vz-0-sessions >/dev/null
    run --separate-stderr vz_example -c kept attach 0 -- cat /etc/hostname
    assert_success
    assert_output "vz-0-sessions"
}

@test "a stopped one keeps its files" {
    kept_container
    vz_example -c kept attach 0 -- touch /tmp/mark
    docker stop vz-0-sessions >/dev/null
    run --separate-stderr vz_example -c kept attach 0 -- test -e /tmp/mark
    assert_success
}

@test "vz new names one" {
    run --separate-stderr vz_example -c kept new api -- cat /etc/hostname
    assert_success
    assert_output "vz-0-sessions-api"
}

@test "attach by name" {
    vz_example -c kept new api -- true
    run --separate-stderr vz_example -c kept attach api -- cat /etc/hostname
    assert_success
    assert_output "vz-0-sessions-api"
}

@test "a name is taken once" {
    vz_example -c kept new api -- true
    run --separate-stderr new_error api
    assert_failure
    assert_output --partial "vz-0-sessions-api exists; attach with \`vz attach api\`"
}

# Two vz runs: the first creates, the second joins and finds its file.
@test "attach: true, the next vz joins the container the first created" {
    vz_example -c shared -- touch /tmp/shared
    run --separate-stderr vz_example -c shared -- test -e /tmp/shared
    assert_success
}

@test "vz kill removes by name" {
    vz_example -c kept new api -- true
    run --separate-stderr vz_example kill api
    assert_success
    assert_output "removed vz-0-sessions-api"
}

@test "vz kill --all removes every container of the repository" {
    kept_container
    vz_example -c kept new api -- true
    vz_example kill --all >/dev/null
    run --separate-stderr vz_example ls
    assert_success
    assert_output "No containers of this repository."
}
