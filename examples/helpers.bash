# Shared by examples/*/test.bats, `load`ed by each: runs vz against the
# example's own configuration files, with a throwaway HOME, from a throwaway
# repository.
#
#   load ../helpers

bats_require_minimum_version 1.5.0
bats_load_library bats-support
bats_load_library bats-assert

# `bats --jobs` runs examples side by side, each one's tests in turn: they share
# its repository, whose container names and state they expect.
export BATS_NO_PARALLELIZE_WITHIN_FILE=true

example_dir=$BATS_TEST_DIRNAME
example=$(basename "$example_dir")
repo_root=$(cd "$example_dir/../.." && pwd)
vz_bin=${VZ:-$repo_root/target/x86_64-unknown-linux-musl/release/viz-shell}
# The stand-in library's image, pinned: the Debian release the base template's
# FROM pins (templates/vz-debian-trixie.Dockerfile), so every run pulls the
# same image.
stand_in_image=debian:trixie-20260918-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a

# The example's configuration files, each read with -f as one of the
# repository's.
configs=()
for file in "$example_dir"/*.vz.yml "$example_dir"/*.vz.yaml; do
    if [[ -f $file ]]; then
        configs+=(-f "$file")
    fi
done

# vz_example ARGS...: vz with the example's configuration files.
vz_example() { "$vz_bin" "${configs[@]}" "$@"; }

# inside COMMAND...: runs COMMAND in a container of this example.
inside() { vz_example -- "$@"; }

# with_config NAME COMMAND...: runs COMMAND in a container of this example's configuration NAME.
with_config() {
    local name=$1
    shift
    vz_example -c "$name" -- "$@"
}

# Once per file: the throwaway home, library and repository, and no
# containers left from an earlier run.
example_setup_file() {
    if [[ ! -x $vz_bin ]]; then
        echo "no vz binary at $vz_bin: run cargo build --release, or set VZ" >&2
        return 1
    fi
    # docker keeps its real configuration: contexts, plugins, logins.
    export DOCKER_CONFIG=${DOCKER_CONFIG:-$HOME/.docker}
    # ~ in a configuration lands in a throwaway home, never in the real one. It
    # lives in the repository, so it has the same path inside a vz container and
    # on the host, whose daemon mounts it. The path is fixed per example, so an
    # image that bakes the home stays cached between runs.
    export HOME=$repo_root/target/vz-examples/$example/home
    rm -rf "$HOME" && mkdir -p "$HOME"
    # A cheap stand-in for the library's template: a configuration named
    # vz-debian-trixie, on a pulled image rather than the base Dockerfile, so a
    # first run writes nothing and no test builds the base. A test that wants
    # another library writes its own.
    mkdir -p "$HOME/.config/viz-shell"
    printf 'name: vz-debian-trixie\nimage: %s\n' "$stand_in_image" \
        > "$HOME/.config/viz-shell/vz-debian-trixie.vz.yml"
    # Every run starts with no state.
    rm -rf "$example_dir/.vz_state"
    export VZ_LOG=${VZ_LOG:-warn}
    # vz runs from a throwaway repository in the throwaway home, never from this
    # one: this repository holds the home, and mounting it would show the home
    # through the repository mount, hiding what a test mounts, removes or bakes.
    # It is named after the example, and so are its containers (vz-0-<example>):
    # examples run side by side, `bats --jobs`, without taking each other's names.
    export app="$HOME/repos/$example"
    git init -q "$app"
    cd "$app"
    # Containers of an earlier, failed run go first.
    vz_example kill --all >/dev/null 2>&1 || true
}

# Before each test: in the throwaway repository, with neither a
# configuration nor a library from the caller's shell.
example_setup() {
    unset VZ_CONFIG XDG_CONFIG_HOME
    cd "$app"
}

# Once per file, whatever passed or failed: this repository's containers go.
example_teardown_file() {
    cd "$app" 2>/dev/null || return 0
    vz_example kill --all >/dev/null 2>&1 || true
}

setup_file() { example_setup_file; }
setup() { example_setup; }
teardown_file() { example_teardown_file; }
