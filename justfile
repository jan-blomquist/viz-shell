set shell := ["bash", "-euo", "pipefail", "-c"]

binary := "target/x86_64-unknown-linux-musl/release/viz-shell"
bin_dir := env_var("HOME") / ".local/bin"

# The base image in images/: where it is published, and its version, from
# images/VERSION. Bump it with every change to the image, and the lines that
# name it; the images workflow publishes on changes under images/ only.
images_registry := "ghcr.io/jan-blomquist"
images_version := trim(read("images/VERSION"))

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

# Build the base image in images/ locally, under its published name, where a
# Dockerfile's FROM finds it without a pull.
build-base-images:
    docker build -t {{images_registry}}/viz-shell-base:{{images_version}} images/base

# Push the built base image to the registry, as the images workflow does.
push-base-images:
    docker push {{images_registry}}/viz-shell-base:{{images_version}}

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
