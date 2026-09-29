#!/usr/bin/env bash
# Mounts show host paths at the same path; the test makes the host side in
# the throwaway home, and runs vz from a repository inside the read-only ~/repos.
source "$(dirname "$0")/../assert.sh"

mkdir -p "$HOME/repos/other" "$HOME/.config/gh"
echo hello > "$HOME/repos/other/readme"
echo "[user]" > "$HOME/.gitconfig"
app="$HOME/repos/app"
git init -q "$app"
cd "$app"

expect_success "the container starts" inside true
expect_output "a read-only mount is readable" "hello" inside cat "$HOME/repos/other/readme"
expect_failure "and not writable" inside touch "$HOME/repos/other/new"
expect_success "the repository inside it stays writable" inside touch "$app/new"
expect_output "and what you create there is yours" "$(id -u)" stat -c %u "$app/new"
expect_success "a :rw mount is writable" inside sh -c 'echo token > ~/.config/gh/hosts.yml'
expect_output "and the host sees the change" "token" cat "$HOME/.config/gh/hosts.yml"
expect_output "a single file can be mounted" "[user]" inside cat "$HOME/.gitconfig"
expect_failure "read-only by default" inside sh -c 'echo x >> ~/.gitconfig'
