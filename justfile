set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Build the static vz binary.
build:
    cargo build --release

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
