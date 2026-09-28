# viz-shell
One shell for every repo. A vz.yml at the repo root declares image, mounts, state, environment, capabilities and sidecars; a host file declares what it grants. One static Rust binary applies both to a Docker or Podman container, on a laptop, an agent host or a CI runner. Linux first, shell first, no editor required
