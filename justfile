set shell := ["bash", "-euo", "pipefail", "-c"]

binary := "target/x86_64-unknown-linux-musl/release/viz-shell"
bin_dir := env_var("HOME") / ".local/bin"

default:
    @just --list

# Build the static viz-shell binary.
build:
    cargo build --release

# Install the built binary on this host, no cargo needed: ~/.local/bin/viz-shell<suffix> and its
# vz<suffix> alias. The suffix defaults to 2 while the legacy viz-shell keeps viz-shell and vz;
# `just install ""` takes the plain names. The first run writes ~/.config/viz-shell/global.yml.
install suffix="2":
    @test -x {{binary}} || { echo "no {{binary}}: run just build first (inside vz, if the host has no cargo)" >&2; exit 1; }
    install -D -m 755 {{binary}} {{bin_dir}}/viz-shell{{suffix}}
    ln -sfn viz-shell{{suffix}} {{bin_dir}}/vz{{suffix}}
    @echo "installed {{bin_dir}}/viz-shell{{suffix}}, alias {{bin_dir}}/vz{{suffix}}"
    @case ":$PATH:" in *":{{bin_dir}}:"*) ;; *) echo "note: {{bin_dir}} is not on PATH" >&2 ;; esac

# Unit tests: no engine needed.
test:
    cargo test

# Run every example's test against the built binary; needs docker.
examples:
    #!/usr/bin/env bash
    set -uo pipefail
    failed=()
    for test in examples/*/test.sh; do
        bash "$test" || failed+=("$(basename "$(dirname "$test")")")
    done
    if (( ${#failed[@]} )); then
        echo "failed: ${failed[*]}" >&2
        exit 1
    fi
    echo "all examples pass"

# Run one example's test: just example state
example name:
    bash examples/{{name}}/test.sh
