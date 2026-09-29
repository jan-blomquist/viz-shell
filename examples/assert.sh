# Shared by examples/*/test.sh. Runs vz against the example's own vz.yml,
# with a throwaway HOME, and offers assertions that stop at the first failure.
#
#   source "$(dirname "$0")/../assert.sh"

set -euo pipefail

example_dir=$(cd "$(dirname "${BASH_SOURCE[1]}")" && pwd)
example=$(basename "$example_dir")
repo_root=$(cd "$example_dir/../.." && pwd)
vz_bin=${VZ:-$repo_root/target/x86_64-unknown-linux-musl/release/vz}
if [[ ! -x $vz_bin ]]; then
    echo "no vz binary at $vz_bin: run cargo build --release, or set VZ" >&2
    exit 1
fi

# docker keeps its real configuration: contexts, plugins, logins.
export DOCKER_CONFIG=${DOCKER_CONFIG:-$HOME/.docker}
# ~ in vz.yml lands in a throwaway home, never in the real one. It lives in
# the repository, so it has the same path inside a vz container and on the
# host, whose daemon mounts it. The path is fixed per example, so an image
# that bakes the home stays cached between runs.
export HOME=$repo_root/target/vz-examples/$example/home
rm -rf "$HOME" && mkdir -p "$HOME"
# Every run starts with no state.
rm -rf "$example_dir/.vz_state"
export VZ_LOG=${VZ_LOG:-warn}
# A profile from the caller's shell would change every test.
unset VZ_PROFILE

cd "$repo_root"
echo "$example"

# inside COMMAND...: runs COMMAND in a container of this example.
inside() { "$vz_bin" -c "$example_dir/vz.yml" -- "$@"; }

# with_profile NAME COMMAND...: runs COMMAND in a container of this example's profile NAME.
with_profile() {
    local name=$1
    shift
    "$vz_bin" -c "$example_dir/vz.yml" --profile "$name" -- "$@"
}

pass() { printf '  \e[32m✓\e[0m %s\n' "$1"; }

# fail WHAT DETAIL...: reports and ends the test.
fail() {
    printf '  \e[31m✗\e[0m %s\n' "$1" >&2
    shift
    printf '      %s\n' "$@" >&2
    exit 1
}

# expect_output WHAT EXPECTED COMMAND...: COMMAND succeeds and prints EXPECTED.
expect_output() {
    local what=$1 expected=$2 actual
    shift 2
    actual=$("$@") || fail "$what" "failed: $*"
    [[ $actual == "$expected" ]] || fail "$what" "expected: $expected" "actual:   $actual"
    pass "$what"
}

# expect_contains WHAT PART COMMAND...: COMMAND succeeds and its output holds PART.
expect_contains() {
    local what=$1 part=$2 actual
    shift 2
    actual=$("$@") || fail "$what" "failed: $*"
    [[ $actual == *"$part"* ]] || fail "$what" "expected to contain: $part" "actual: $actual"
    pass "$what"
}

# expect_lacks WHAT PART COMMAND...: COMMAND succeeds and its output lacks PART.
expect_lacks() {
    local what=$1 part=$2 actual
    shift 2
    actual=$("$@") || fail "$what" "failed: $*"
    [[ $actual != *"$part"* ]] || fail "$what" "expected not to contain: $part" "actual: $actual"
    pass "$what"
}

expect_success() {
    local what=$1
    shift
    "$@" >/dev/null || fail "$what" "failed: $*"
    pass "$what"
}

expect_failure() {
    local what=$1
    shift
    if "$@" >/dev/null 2>&1; then
        fail "$what" "succeeded: $*"
    fi
    pass "$what"
}

# expect_status WHAT STATUS COMMAND...: COMMAND exits with STATUS.
expect_status() {
    local what=$1 expected=$2 status=0
    shift 2
    "$@" >/dev/null 2>&1 || status=$?
    [[ $status == "$expected" ]] || fail "$what" "expected exit $expected, got $status"
    pass "$what"
}
