#!/usr/bin/env bash
# A local overlay: vz.local.yml over vz.yml, root and profile alike.
source "$(dirname "$0")/../assert.sh"
# This example's local file is tracked on purpose: its warning, checked last, stays out of the rest.
export VZ_LOG=error

mkdir -p "$HOME/shared" "$HOME/mine"
echo mine > "$HOME/mine/note"

expect_output "the local value overrides the repository's" "local" inside sh -c 'echo "$WHO"'
expect_success "the local mode wins: ~/shared is writable" inside touch "$HOME/shared/new"
expect_output "the local file adds its own mount" "mine" inside cat "$HOME/mine/note"
expect_output "a local profile merges over the repository's" "local-extra" \
    with_profile extra sh -c 'echo "$WHO"'
expect_output "a profile only the local file defines is selectable" "mine" \
    with_profile mine sh -c 'echo "$WHO"'
show_config() { "$vz_bin" -c "$example_dir/vz.yml" --show-effective-config; }
expect_contains "--show-effective-config names the local file" "local $example_dir/vz.local.yml" show_config
expect_contains "and lists its layers" "local root" show_config
tracked_warning() { VZ_LOG=warn inside true 2>&1 >/dev/null; }
expect_contains "a tracked local file is used, with a warning" \
    "vz.local.yml is tracked by git: it is meant to be personal" tracked_warning
