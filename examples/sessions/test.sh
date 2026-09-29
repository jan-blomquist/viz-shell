#!/usr/bin/env bash
# Named containers; persistent ones, attached to by index, name, or `attach: true`.
source "$(dirname "$0")/../assert.sh"

vz() { "$vz_bin" -c "$example_dir/vz.yml" "$@"; }
# vz ls without its header, in single spaces: name, profile, persistent, state.
ls_rows() { vz ls | awk 'NR > 1 { print $1, $2, $3, $4 }'; }
attach_error() { vz attach 0 -- true 2>&1 >/dev/null || true; }

# Containers of an earlier, failed run go first; this run's go at the end.
vz kill --all >/dev/null
trap 'vz kill --all >/dev/null 2>&1 || true' EXIT

expect_output "no containers yet" "No containers of this repository." vz ls
expect_output "a container is named, its hostname too" "vz-0-app" inside cat /etc/hostname
expect_output "and removed on exit" "No containers of this repository." vz ls

expect_success "persistent: the container outlives its command" vz --profile kept -- true
expect_output "vz ls lists it, running" "vz-0-app kept yes running" ls_rows
expect_output "the next container takes the next free index" "vz-1-app" inside cat /etc/hostname

expect_failure "attaching across profiles is refused" vz attach 0 -- true
expect_contains "naming the command to use" "vz --profile kept attach 0" attach_error
expect_output "attach by index, as you" "$(id -un)@vz-0-app" \
    vz --profile kept attach 0 -- sh -c 'echo "$(id -un)@$(cat /etc/hostname)"'
expect_success "a file written in the container" vz --profile kept attach 0 -- touch /tmp/mark
expect_success "is there on the next attach" vz --profile kept attach 0 -- test -e /tmp/mark
expect_output "attach alone: the only running container" "vz-0-app" \
    vz --profile kept attach -- cat /etc/hostname

docker stop vz-0-app >/dev/null
expect_output "a stopped one is started on attach" "vz-0-app" \
    vz --profile kept attach 0 -- cat /etc/hostname
expect_success "with its files" vz --profile kept attach 0 -- test -e /tmp/mark

expect_output "vz new names one" "vz-1-app-api" vz --profile kept new api -- cat /etc/hostname
expect_output "attach by name" "vz-1-app-api" vz --profile kept attach api -- cat /etc/hostname
expect_failure "a name is taken once" vz --profile kept new api -- true

expect_success "attach: true, the first vz creates" vz --profile shared -- touch /tmp/shared
expect_success "the next joins it" vz --profile shared -- test -e /tmp/shared

expect_output "vz kill removes by name" "removed vz-1-app-api" vz kill api
expect_success "vz kill --all removes the rest" vz kill --all
expect_output "and none are left" "No containers of this repository." vz ls
