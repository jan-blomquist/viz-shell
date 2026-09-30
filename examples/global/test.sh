#!/usr/bin/env bash
# The global configuration: what every repository starts from, a repository's
# own configuration on top, and secrets only the trusted profile sees.
source "$(dirname "$0")/../assert.sh"

config_home="$HOME/.config/viz-shell"
mkdir -p "$config_home"
cp "$example_dir/viz-shell.global.yml" "$config_home/viz-shell.global.yml"
printf 'EVERYWHERE=yes\n' > "$config_home/environment"
printf 'TRUSTED_SECRET=s3cret\n' > "$config_home/trusted.env"

repo="$HOME/repos/bare"
git init -q "$repo"
cd "$repo"
run_vz() { "$vz_bin" "$@"; }

expect_output "a repository without configuration starts from the global one" "global" \
    run_vz -- sh -c 'echo "$WHO"'
expect_output "the global file's relative paths are its own" "yes" \
    run_vz -- sh -c 'echo "$EVERYWHERE"'
expect_output "trusted secrets stay out of the default mode" "" \
    run_vz -- sh -c 'echo "$TRUSTED_SECRET"'
expect_output "and come in with --profile trusted" "s3cret" \
    run_vz --profile trusted -- sh -c 'echo "$TRUSTED_SECRET"'

cat > vz.yml <<'YAML'
env:
  defaults:
    WHO: repo
profiles:
  trusted:
    env:
      defaults:
        WHO: repo-trusted
  ci:
    extends: trusted
YAML

expect_output "the repository's root overrides the global root" "repo" \
    run_vz -- sh -c 'echo "$WHO"'
expect_output "a profile in both files: the global section, then the repo's" "repo-trusted s3cret" \
    run_vz --profile trusted -- sh -c 'echo "$WHO $TRUSTED_SECRET"'
expect_output "a repository profile extends a global one" "s3cret" \
    run_vz --profile ci -- sh -c 'echo "$TRUSTED_SECRET"'
expect_contains "vz profiles lists where each is defined" "trusted   global, repo" run_vz profiles
expect_contains "vz profiles lists what each extends" "ci        repo           trusted" run_vz profiles

mv "$config_home/viz-shell.global.yml" "$config_home/global.yml"
startup_logs() { VZ_LOG=viz_shell=info run_vz -- sh -c 'echo "$TRUSTED_SECRET"' 2>&1; }
expect_contains "the former name, global.yml, is still read" \
    "reading global.yml; the name is viz-shell.global.yml now" startup_logs
