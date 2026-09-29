# viz-shell
One shell for every repo. A vz.yml at the repo root declares image, mounts, state, environment, 
capabilities and sidecars; a host file declares what it grants. 
One static Rust binary applies both to a Docker or Podman container, on a laptop, an agent host or a CI runner. 
Linux first, shell first, no editor required

> Early MVP: `vz` runs the image named in `vz.yml` to completion and prints its output.

## Build

Rust 1.95.0 and the musl target are pinned in `rust-toolchain.toml`; rustup installs them.

```sh
cargo build --release
```

The binary is static: `target/x86_64-unknown-linux-musl/release/vz`.
Build it inside a container, run it on any Linux host.

## Use

`vz.yml` in the current directory:

```yaml
image: hello-world
```

```sh
vz    # pulls the image if missing, runs it, removes the container
```

`vz` exits with the container's exit code.

Unknown keys in `vz.yml` are refused, naming the line.

## Logging

Logs go to stderr, filtered by `VZ_LOG` (default `warn,vz=info`):

```sh
VZ_LOG=vz=debug vz    # each engine step
VZ_LOG=debug vz       # includes bollard and hyper
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

