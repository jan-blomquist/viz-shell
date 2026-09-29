#!/usr/bin/env bash
# You are you inside: same user, ids, working directory and exit code.
source "$(dirname "$0")/../assert.sh"

expect_success "the container starts" inside true
expect_output "you are you" "$(id -un)" inside id -un
expect_output "with your uid and gid" "$(id -u):$(id -g)" inside sh -c 'echo "$(id -u):$(id -g)"'
expect_output "your home is the host's home path" "$HOME" inside sh -c 'echo ~'
expect_output "in the directory vz ran from" "$PWD" inside pwd

scratch="$PWD/scratch"
mkdir -p "$scratch"
expect_success "the repository is writable" inside touch "$scratch/created"
expect_output "what you create is yours on the host" "$(id -u)" stat -c %u "$scratch/created"

expect_status "the command's exit code passes through" 3 inside sh -c 'exit 3'
