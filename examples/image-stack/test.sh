#!/usr/bin/env bash
# Image stacking: `ARG BASE` builds on the image below; anything else replaces it.
source "$(dirname "$0")/../assert.sh"

markers() { with_profile "$1" sh -c 'echo $(ls /etc/stack)'; }
expect_output "the default profile builds base.Dockerfile" "base" inside sh -c 'echo $(ls /etc/stack)'
expect_output "a Dockerfile with ARG BASE stacks on it" "base tools" markers tools
expect_output "one without ARG BASE replaces it" "alone" markers alone
expect_failure "an image reference replaces it" with_profile pulled test -e /etc/stack
expect_output "a Dockerfile with ARG BASE stacks on a reference" "tools" markers pulled-tools
expect_output "a replacing Dockerfile on top of a stack leaves only itself" "alone" markers deep

show_config() { "$vz_bin" -c "$example_dir/vz.yml" --profile "$1" --show-effective-config; }
expect_contains "--show-effective-config names the base" \
    "# image: $example_dir/base.Dockerfile (repository default)" show_config tools
expect_contains "and says the top stacks" \
    "# image: $example_dir/tools.Dockerfile (repository tools, ARG BASE: stacks)" show_config tools
expect_contains "or replaces" "# image: debian:stable-slim (repository pulled, replaces)" show_config pulled

images() { docker image ls --format '{{.Repository}}'; }
expect_contains "built images are named after the Dockerfiles' folder" "vz-image-stack" images

startup_logs() { VZ_LOG=viz_shell=info with_profile tools true 2>&1 >/dev/null; }
expect_lacks "a second run builds nothing" "building" startup_logs
