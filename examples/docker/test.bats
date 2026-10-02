#!/usr/bin/env bats
# The host's docker daemon, shared: reachable inside as you, off in another configuration.

load ../helpers

setup_file() {
    example_setup_file
    endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
    socket=${endpoint#unix://}
    export endpoint socket_gid=$(stat -c %g "$socket")
}

# docker's error inside, on stdout.
docker_ps_offline() { with_config offline sh -c 'docker ps 2>&1'; }
in_socket_group() { inside sh -c "id -G | tr ' ' '\n' | grep -qx $socket_gid"; }

@test "the container starts" {
    run --separate-stderr inside true
    assert_success
}

@test "docker inside reaches the host's daemon" {
    run --separate-stderr inside docker version --format '{{.Server.Version}}'
    assert_success
    assert_output "$(docker version --format '{{.Server.Version}}')"
}

@test "as you, without sudo" {
    run --separate-stderr inside docker ps
    assert_success
}

@test "the socket is found through DOCKER_HOST" {
    run --separate-stderr inside sh -c 'echo "$DOCKER_HOST"'
    assert_success
    assert_output "$endpoint"
}

@test "you are in the socket's group" {
    run --separate-stderr in_socket_group
    assert_success
}

@test "another configuration turns it off: no socket to reach" {
    run --separate-stderr docker_ps_offline
    assert_failure
    assert_output --partial "docker.sock"
}
