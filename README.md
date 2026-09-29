# viz-shell
One shell for every repo. A vz.yml at the repo root declares image, mounts, state, environment, 
capabilities and sidecars; a host file declares what it grants. 
One static Rust binary applies both to a Docker or Podman container, on a laptop, an agent host or a CI runner. 
Linux first, shell first, no editor required

> Early MVP: `vz` pulls or builds the image in `vz.yml`, runs it to completion and prints its output.

## Build

Rust 1.95.0 and the musl target are pinned in `rust-toolchain.toml`; rustup installs them.

```sh
cargo build --release
```

The binary is static: `target/x86_64-unknown-linux-musl/release/vz`.
Build it inside a container, run it on any Linux host.

## Use

`vz.yml` in the current directory names an image to pull:

```yaml
image: hello-world
```

or a Dockerfile to build:

```yaml
image:
  dockerfile: Dockerfile
  context: .                  # optional, default: the current directory
  args: { GREETING: hello }   # optional build args
```

```sh
vz    # pulls or builds the image if missing, runs it, removes the container
```

`vz` exits with the container's exit code.

A built image is tagged `vz-<directory>:<hash of the Dockerfile and args>`,
and builds only when that tag is missing. Editing the Dockerfile or args rebuilds;
editing a file the Dockerfile copies does not. Remove the image to force a rebuild.
`vz` drives the engine through the `docker` CLI, via
[docker-wrapper](https://github.com/joshrotenberg/docker-wrapper),
so builds honour `.dockerignore`.

This repository's own `Dockerfile` is the Rust toolchain with `vz` built from the checkout.

Unknown keys in `vz.yml` are refused, naming the line.

## Logging

Logs go to stderr, filtered by `VZ_LOG` (default `warn,vz=info`):

```sh
VZ_LOG=vz=debug vz    # each step, and every docker command in full
VZ_LOG=debug vz       # also docker-wrapper's spans: each CLI call, its exit code and output size
```

## Test

```sh
cargo test                 # unit tests, no engine needed
cargo test -- --ignored    # needs a Docker engine
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

