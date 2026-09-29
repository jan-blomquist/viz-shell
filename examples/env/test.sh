#!/usr/bin/env bash
# The environment's sources and their order, and that values stay out of sight.
source "$(dirname "$0")/../assert.sh"

vz=("$vz_bin" -c "$example_dir/vz.yml")
printf 'LEVEL="first file"\nFROM_FILE=yes\n' > "$HOME/example.env"
printf 'LEVEL="ci file"\n' > "$HOME/ci.env"
show() { inside sh -c "echo \"\$$1\""; }

expect_output "a default" "hello" show GREETING
expect_output "\${repo} is substituted" "$repo_root" show REPO_ROOT
expect_output "a file beats a default" "first file" show LEVEL
expect_output "and adds its own" "yes" show FROM_FILE

printf 'LEVEL="second file"\n' > "$HOME/local.env"
expect_output "a later file beats an earlier one" "second file" show LEVEL
rm "$HOME/local.env"
expect_output "a bare path may be missing" "first file" show LEVEL

expect_output "passthrough beats files" "host" \
    env LEVEL=host "${vz[@]}" -- sh -c 'echo "$LEVEL"'
expect_output "--env beats everything" "cli" \
    env LEVEL=host "${vz[@]}" --env LEVEL=cli -- sh -c 'echo "$LEVEL"'
expect_output "a passthrough glob" "globbed" \
    env VZ_EXAMPLE_GLOB_ONE=globbed "${vz[@]}" -- sh -c 'echo "$VZ_EXAMPLE_GLOB_ONE"'

expect_output "profile: null removes a default" "" with_profile ci sh -c 'echo "$GREETING"'
expect_output "profile: its file comes after the root's" "ci file" with_profile ci sh -c 'echo "$LEVEL"'
expect_output "profile: adds a passthrough" "from ci" \
    env VZ_EXAMPLE_CI="from ci" "${vz[@]}" --profile ci -- sh -c 'echo "$VZ_EXAMPLE_CI"'
expect_output "which the root does not have" "" \
    env VZ_EXAMPLE_CI="from ci" "${vz[@]}" -- sh -c 'echo "$VZ_EXAMPLE_CI"'
expect_output "profile: enabled: false stops a passthrough" "" \
    env VZ_EXAMPLE_TOKEN=secret "${vz[@]}" --profile ci -- sh -c 'echo "$VZ_EXAMPLE_TOKEN"'

show_env() { env VZ_EXAMPLE_TOKEN=secret-value "${vz[@]}" --show-env; }
expect_contains "--show-env names each source" "env.files $HOME/example.env" show_env
expect_lacks "and never a value" "secret-value" show_env

logged_command() {
    env VZ_EXAMPLE_TOKEN=secret-value VZ_LOG=vz=debug "${vz[@]}" -- true 2>&1 >/dev/null
}
expect_contains "docker gets the name" "--env VZ_EXAMPLE_TOKEN" logged_command
expect_lacks "never the value" "secret-value" logged_command

missing_required() { rm "$HOME/example.env"; inside true 2>&1; }
expect_failure "a missing required: true file is refused" missing_required
