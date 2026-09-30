#!/usr/bin/env bash
# Hooks: create once per container, attach before every entry, in order.
source "$(dirname "$0")/../assert.sh"

vz() { "$vz_bin" -c "$example_dir/vz.yml" "$@"; }
log=.local/state/hooks/log
# stderr of a failing entry.
failure() { vz --profile "$1" -- true 2>&1 >/dev/null || true; }

# Containers of an earlier, failed run go first; this run's go at the end.
vz kill --all >/dev/null
trap 'vz kill --all >/dev/null 2>&1 || true' EXIT

expect_output "create in order, then attach, once each" $'create\nsecond\nattach' \
    inside sh -c "cat ~/$log"
expect_output "a fresh container runs them again" $'create\nsecond\nattach' \
    inside sh -c "cat ~/$log"
expect_contains "a hook's output reaches the terminal" "hello from create" \
    with_profile loud true
expect_output "enabled: false removes one" $'create\nattach' \
    with_profile quiet sh -c "cat ~/$log"
mkdir sub
expect_output "hooks run in the repository root" "$app" \
    bash -c 'cd sub && "$@"' _ "$vz_bin" -c "$example_dir/vz.yml" -- sh -c 'cat ~/.local/state/hooks/pwd'

expect_failure "a failing hook fails the entry" vz --profile failing -- true
expect_contains "naming the hook" 'create hook `exit 3`' failure failing
expect_contains "and its exit status" "exit status 3" failure failing

expect_contains "persistent: a create hook's output reaches the terminal" "hello from create" \
    vz --profile kept -- true
expect_output "create ran once, attach on each entry" $'create\nsecond\nattach\nattach' \
    vz --profile kept attach 0 -- sh -c "cat ~/$log"
docker stop vz-0-app >/dev/null
expect_output "a stop and start runs create no more" $'create\nsecond\nattach\nattach\nattach' \
    vz --profile kept attach 0 -- sh -c "cat ~/$log"
expect_contains "persistent: a failing hook names itself" 'create hook `exit 3`: exit status 3' \
    failure kept-failing
