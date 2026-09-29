#!/usr/bin/env bash
# shell: fish when no command is given; a warning and a fallback without it.
source "$(dirname "$0")/../assert.sh"

# shell_runs SCRIPT [PROFILE]: the interactive shell, reading SCRIPT from stdin.
shell_runs() {
    echo "$1" | "$vz_bin" -c "$example_dir/vz.yml" ${2:+--profile "$2"}
}
login_shell() { inside sh -c 'grep "^$(id -un):" /etc/passwd | cut -d: -f7'; }
missing_logs() { shell_runs 'true' missing 2>&1 >/dev/null; }

expect_success "the image builds and starts" inside true
expect_output "no command: fish" "fish" shell_runs 'status fish-path | xargs basename'
expect_output "SHELL names it, for commands too" "/usr/bin/fish" inside sh -c 'echo $SHELL'
expect_output "and so does the user's passwd entry" "/usr/bin/fish" login_shell
expect_success "fish's configuration is state" shell_runs 'set -U greeting hello'
expect_output "and outlives the container" "hello" shell_runs 'echo $greeting'
expect_output "an absolute path, used as it is" "/usr/bin/fish" \
    with_profile absolute sh -c 'echo $SHELL'
expect_output "missing from the image: sh instead" "/bin/sh" \
    shell_runs 'echo $SHELL' missing
expect_contains "with a warning" "shell \`zsh\` is not in the image" missing_logs
