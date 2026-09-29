set shell := ["bash", "-euo", "pipefail", "-c"]

binary := "target/x86_64-unknown-linux-musl/release/viz-shell"
bin_dir := env_var("HOME") / ".local/bin"

# The base images in images/: where they will be published, and their version.
# Bump the version with every change to them, and the FROM lines that name it.
images_registry := "ghcr.io/jan-blomquist"
images_version := "2026.9.1"

default:
    @just --list

# Build the static viz-shell binary.
build:
    cargo build --release

# Install the built binary on this host, no cargo needed: ~/.local/bin/viz-shell and its vz
# alias. The first run writes ~/.config/viz-shell/global.yml.
install:
    @test -x {{binary}} || { echo "no {{binary}}: run just build first (inside vz, if the host has no cargo)" >&2; exit 1; }
    install -D -m 755 {{binary}} {{bin_dir}}/viz-shell
    ln -sfn viz-shell {{bin_dir}}/vz
    @echo "installed {{bin_dir}}/viz-shell, alias {{bin_dir}}/vz"
    @case ":$PATH:" in *":{{bin_dir}}:"*) ;; *) echo "note: {{bin_dir}} is not on PATH" >&2 ;; esac

# viz-shell-base first, then viz-shell-agents from it. A Dockerfile's FROM finds
# them without a pull.
# Build the base images in images/ locally, under their published names.
build-base-images:
    docker build -t {{images_registry}}/viz-shell-base:{{images_version}} images/base
    docker build -t {{images_registry}}/viz-shell-agents:{{images_version}} \
        --build-arg BASE={{images_registry}}/viz-shell-base:{{images_version}} images/agents

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
