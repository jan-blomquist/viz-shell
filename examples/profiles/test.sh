#!/usr/bin/env bash
# A profile changes, removes or adds entries on top of the root.
source "$(dirname "$0")/../assert.sh"

mkdir -p "$HOME/repos/other"
echo hello > "$HOME/repos/other/readme"

# with_profile NAME COMMAND...: runs COMMAND in a container of this example's profile NAME.
with_profile() {
    local name=$1
    shift
    "$vz_bin" -c "$example_dir/vz.yml" --profile "$name" -- "$@"
}

expect_output "the root mounts ~/repos" "hello" inside cat "$HOME/repos/other/readme"
expect_failure "read-only" inside touch "$HOME/repos/other/new"

expect_success "a profile overrides the mode: writable" with_profile writable touch "$HOME/repos/other/new"

expect_failure "false removes the mount: isolated" with_profile isolated test -e "$HOME/repos/other/readme"

expect_success "extends builds on another profile: scratch" \
    with_profile scratch sh -c "! test -e ~/repos/other/readme && echo kept > ~/scratch/probe"
expect_output "and adds its own state" "kept" with_profile scratch cat "$HOME/scratch/probe"

select_by_env() { VZ_PROFILE=isolated "$vz_bin" -c "$example_dir/vz.yml" -- "$@"; }
expect_failure "VZ_PROFILE selects a profile too" select_by_env test -e "$HOME/repos/other/readme"

expect_failure "an unknown profile is refused" with_profile nope true
