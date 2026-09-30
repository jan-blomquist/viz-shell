#!/usr/bin/env bash
# Mounts show host paths inside, at the same path or at a target; the test
# makes the host side in the throwaway home, and runs vz from a repository
# inside the read-only ~/repos.
source "$(dirname "$0")/../assert.sh"

mkdir -p "$HOME/repos/other" "$HOME/.config/gh" "$HOME/library"
echo hello > "$HOME/repos/other/readme"
echo shelf > "$HOME/library/book"
echo "[user]" > "$HOME/.gitconfig"

expect_success "the container starts" inside true
skip_log() { VZ_LOG=viz_shell=debug inside true 2>&1 >/dev/null; }
expect_contains "a mount on the repository is skipped, and says so" \
    "skipping mount $app: it lands on the repository" skip_log
expect_output "a read-only mount is readable" "hello" inside cat "$HOME/repos/other/readme"
expect_failure "and not writable" inside touch "$HOME/repos/other/new"
expect_success "the repository inside it stays writable" inside touch "$app/new"
expect_output "and what you create there is yours" "$(id -u)" stat -c %u "$app/new"
expect_success "a bare mount is writable" inside sh -c 'echo token > ~/.config/gh/hosts.yml'
expect_output "and the host sees the change" "token" cat "$HOME/.config/gh/hosts.yml"
expect_output "a single file can be mounted" "[user]" inside cat "$HOME/.gitconfig"
expect_failure "and read-only with :ro" inside sh -c 'echo x >> ~/.gitconfig'
expect_output "a mount with a target shows the host folder there" "shelf" \
    inside cat "$HOME/.agents/library/book"
expect_output "the folder around it is yours" "$(id -u)" inside stat -c %u "$HOME/.agents"
expect_output "one source lands at several targets" "shelf" \
    inside cat "$HOME/.config/agent/library/book"
expect_success "the state folder around a mount stays writable" \
    inside touch "$HOME/.config/agent/notes"
expect_output "and the mount point vz made in it is yours" "$(id -u)" \
    stat -c %u "$app/.vz_state$HOME/.config/agent/library"
