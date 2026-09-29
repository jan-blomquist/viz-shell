#!/usr/bin/env bash
# The host's docker daemon, shared: reachable inside as you, off in a profile.
source "$(dirname "$0")/../assert.sh"

endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
socket=${endpoint#unix://}
socket_gid=$(stat -c %g "$socket")

expect_success "the container starts" inside true
expect_output "docker inside reaches the host's daemon" \
    "$(docker version --format '{{.Server.Version}}')" \
    inside docker version --format '{{.Server.Version}}'
expect_success "as you, without sudo" inside docker ps
expect_output "the socket is found through DOCKER_HOST" "$endpoint" inside sh -c 'echo "$DOCKER_HOST"'

in_socket_group() { inside sh -c "id -G | tr ' ' '\n' | grep -qx $socket_gid"; }
expect_success "you are in the socket's group" in_socket_group

expect_failure "a profile turns it off" with_profile offline docker ps
