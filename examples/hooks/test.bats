#!/usr/bin/env bats
# Hooks: create once per container, attach before every entry, in order. Each
# test starts with no containers of this repository: it makes those it needs.

load ../helpers

log=.local/state/hooks/log
# stderr of an entry that vz refuses.
refused() { vz_example -c "$1" -- true 2>&1 >/dev/null; }
from_sub() {
    mkdir -p sub
    (cd sub && "$vz_bin" -f "$example_dir/default.vz.yml" -- sh -c 'cat ~/.local/state/hooks/pwd')
}

setup() {
    example_setup
    vz_example kill --all >/dev/null 2>&1 || true
}

# A first container before it: the log is the container's, not carried over.
@test "a fresh container runs create in order, then attach, once each" {
    inside true
    run --separate-stderr inside sh -c "cat ~/$log"
    assert_success
    assert_output $'create\nsecond\nattach'
}

@test "a hook's output reaches the terminal" {
    run --separate-stderr with_config loud true
    assert_success
    assert_output --partial "hello from create"
}

@test "enabled: false removes one" {
    run --separate-stderr with_config quiet sh -c "cat ~/$log"
    assert_success
    assert_output $'create\nattach'
}

@test "hooks run in the repository root" {
    run --separate-stderr from_sub
    assert_success
    assert_output "$app"
}

@test "a failing hook fails the entry, naming the hook and its exit status" {
    run --separate-stderr refused failing
    assert_failure
    assert_output --partial 'create hook `exit 3`: exit status 3'
}

@test "persistent: a create hook's output reaches the terminal" {
    run --separate-stderr vz_example -c kept -- true
    assert_success
    assert_output --partial "hello from create"
}

@test "persistent: attach 0 runs attach again, create no more" {
    vz_example -c kept -- true
    run --separate-stderr vz_example -c kept attach 0 -- sh -c "cat ~/$log"
    assert_success
    assert_output $'create\nsecond\nattach\nattach'
}

@test "persistent: a stop and start runs create no more" {
    vz_example -c kept -- true
    docker stop vz-0-hooks >/dev/null
    run --separate-stderr vz_example -c kept attach 0 -- sh -c "cat ~/$log"
    assert_success
    assert_output $'create\nsecond\nattach\nattach'
}

# UNRUN: written in the audit pass, verify with just examples hooks
@test "attach: true, a second vz joins the container: attach runs again, create no more" {
    with_config joined true
    run --separate-stderr with_config joined sh -c "cat ~/$log"
    assert_success
    assert_output $'create\nsecond\nattach\nattach'
}

@test "persistent: a failing hook fails the entry, naming itself" {
    run --separate-stderr refused kept-failing
    assert_failure
    assert_output --partial 'create hook `exit 3`: exit status 3'
}
