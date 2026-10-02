set shell := ["bash", "-euo", "pipefail", "-c"]

binary := "target/x86_64-unknown-linux-musl/release/viz-shell"
bin_dir := env_var("HOME") / ".local/bin"

default:
    @just --list

# Build the static viz-shell binary.
build:
    cargo build --release

# Install the built binary on this host, no cargo needed: ~/.local/bin/viz-shell and its vz
# alias. The first run writes the library, ~/.config/viz-shell/: vz-debian-trixie.vz.yml and its Dockerfile.
install:
    @test -x {{binary}} || { echo "no {{binary}}: run just build first (inside vz, if the host has no cargo)" >&2; exit 1; }
    install -D -m 755 {{binary}} {{bin_dir}}/viz-shell
    ln -sfn viz-shell {{bin_dir}}/vz
    @echo "installed {{bin_dir}}/viz-shell, alias {{bin_dir}}/vz"
    @case ":$PATH:" in *":{{bin_dir}}:"*) ;; *) echo "note: {{bin_dir}} is not on PATH" >&2 ;; esac

# Unit tests: no engine needed.
test:
    cargo test

# Example tests, examples/*/test.bats, against the built binary; need docker; inside vz: `vz -- just examples`.
examples filter="":
    #!/usr/bin/env bash
    set -euo pipefail
    binary=${VZ:-{{binary}}}
    test -x "$binary" || { echo "no $binary: run just build first" >&2; exit 1; }
    if [[ -n "{{filter}}" ]]; then
        bats --pretty "examples/{{filter}}/test.bats"
    else
        bats --recursive --pretty examples
    fi
