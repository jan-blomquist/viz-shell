# viz-shell
One shell for every repo. A vz.yml at the repo root declares image, mounts, state, environment, 
capabilities and sidecars; a host file declares what it grants. 
One static Rust binary applies both to a Docker or Podman container, on a laptop, an agent host or a CI runner. 
Linux first, shell first, no editor required

> Early MVP: `vz` opens a shell, as you, in the image `vz.yml` names, with the repository mounted.

- [Build](#build)
- [Use](#use)
- [Images](#images)
- [Your user in the image](#your-user-in-the-image)
- [State](#state)
- [Mounts](#mounts)
- [Examples](#examples)
- [Logging](#logging)
- [Test](#test)
- [License](#license)

## Build

```sh
cargo build --release    # → target/x86_64-unknown-linux-musl/release/vz
```

Rust 1.95.0 and the musl target are pinned in `rust-toolchain.toml`. The binary is static,
because `vz` mounts itself into every container it starts: build it anywhere, run it on any Linux host.

## Use

```sh
vz                  # a shell: bash, else sh
vz -- cargo test    # one command instead
vz -c other.yml     # another configuration: -c, --config-file
```

`vz` reads `vz.yml` at the git root, or the `-c` file, pulls or builds the image if missing, and runs a container
that is removed on exit; `vz` exits with its exit code. Inside:

- the repository is mounted read-write at its host path; the working directory is yours;
- you are you: same user, uid, group and home, so `whoami`, `~` and ssh work as on the host;
- `TERM`, `COLORTERM`, `LANG` and `VZ_LOG` are copied in when set.

Unknown keys in `vz.yml` are refused, naming the line.

## Images

```yaml
image: hello-world            # pull
```

```yaml
image:                        # build; paths relative to the vz.yml's folder
  dockerfile: Dockerfile
  context: .                  # optional, default: the vz.yml's folder
  args: { GREETING: hello }   # optional
```

- A built image is tagged `vz-<dir>:<hash of Dockerfile + args>` and builds only when missing.
  Editing the Dockerfile or args rebuilds; editing a copied file does not — remove the image to force it.
- Builds run `docker build`, via [docker-wrapper](https://github.com/joshrotenberg/docker-wrapper),
  so `.dockerignore` applies.
- This repository's `Dockerfile`: the Rust toolchain, with `vz` built from the checkout.

## Your user in the image

**Default — added at start.** The image needs no user. `vz` starts the container as root with
itself as entrypoint, which adds your `/etc/passwd` and `/etc/group` lines, creates your home,
then becomes you. One image serves everyone; your home starts empty.

**Opt-in — baked at build.** For tools installed into your home or files owned by you, declare
any of these build args; `vz` passes your values:

| Arg | Value | | Arg | Value |
|---|---|---|---|---|
| `VZ_USER` | user name | | `VZ_GROUP` | group name |
| `VZ_UID` | uid | | `VZ_HOME` | home path |
| `VZ_GID` | primary gid | | | |

```dockerfile
ARG VZ_USER
ARG VZ_UID
ARG VZ_GID
ARG VZ_GROUP
ARG VZ_HOME
RUN groupadd -g "$VZ_GID" "$VZ_GROUP" \
 && useradd -u "$VZ_UID" -g "$VZ_GID" -d "$VZ_HOME" -m -s /bin/bash "$VZ_USER"
# Later build steps run as you.
USER $VZ_USER
```

Alpine: `addgroup -g "$VZ_GID" "$VZ_GROUP" && adduser -D -u "$VZ_UID" -G "$VZ_GROUP" -h "$VZ_HOME" "$VZ_USER"`.

- Declared args join the image hash, so such an image is built per user.
- At start, a baked user must match you: name, uid and home; group name and gid.
  Anything else holding your name, uid or gid is refused, naming it.
- Always pass `-d "$VZ_HOME"`. On Ubuntu 23.04+, `userdel -r ubuntu` first: it holds uid 1000.
- `USER` affects only the build; the container always starts as root for the entrypoint.
- Don't set these in `vz.yml` `args`: they would override yours and fail the match.

## State

Container paths whose contents survive the container. Each is kept in the state folder, `.vz_state/`
at the git root by default, at its own container path and mounted back; the host's own files are untouched.

```yaml
state_dir: .vz_state                                 # optional: the state folder, see below
state:
  - ~/.claude                                        # folder: the short form
  - /var/cache/apt                                   # any absolute path
  - { path: ~/.claude.json, type: file, init: "{}" } # file, created with "{}" the first time
```

| Entry | Inside | Kept at |
|---|---|---|
| `~/.claude` | `/home/sally/.claude` | `.vz_state/home/sally/.claude` |
| `/var/cache/apt` | `/var/cache/apt` | `.vz_state/var/cache/apt` |

- `type`: `dir` (default) or `file`. `init`: a file's content when `vz` creates it; never rewritten.
- `state_dir`: relative to the `vz.yml`'s folder, `~/…` or absolute. Without it every configuration,
  `-c` ones included, shares `.vz_state/` at the git root.
- `vz` creates missing entries as you. Delete the state folder to start over; ignoring it in git is up to you,
  but keep it out of Docker build contexts: `**/.vz_state/` in `.dockerignore`.
- A state folder hides what the image had at that path, and belongs to you.
- A `file` suits tools that update in place. One that saves by rename (`git config`) fails with
  "Device or resource busy": keep a folder and point the tool into it.
- Paths start with `~/` or `/`, without `.`, `..` or `//`, and may not hold or sit inside the repository.

## Mounts

Host paths shown at the same path inside, reusing the host's own files.

```yaml
mounts:
  - ~/repos            # read-only, the default
  - ~/.gitconfig:ro    # read-only, spelled out
  - ~/.config/gh:rw    # read-write
```

The suffix follows docker's `-v`; any other suffix is refused. The full form works too:
`- { path: ~/.config/gh, mode: rw }`, or as a block mapping.

- The repository `vz` runs for is always read-write, even inside a read-only mount like `~/repos`:
  deeper mounts land on top.
- A mount must exist on the host; `vz` never creates one.
- Paths follow the state rules; a mount may not overlap a state path.

## Examples

Recipes in [`examples/`](examples), each with a `test.sh` that runs it and checks the result.
Copy a folder's `vz.yml` and `Dockerfile` to your repository root, or try one in place with `-c`.

| Recipe | Shows |
|---|---|
| [`pull-image`](examples/pull-image) | the smallest `vz.yml` |
| [`build-dockerfile`](examples/build-dockerfile) | building from a Dockerfile, with args |
| [`baked-user`](examples/baked-user) | your user baked into the image, installing into your home |
| [`state`](examples/state) | folders, a file with `init`, absolute paths |
| [`mounts`](examples/mounts) | read-only `~/repos`, a read-write config folder, a single file |

## Logging

Stderr, filtered by `VZ_LOG` (default `warn,vz=info`):

```sh
VZ_LOG=vz=debug vz    # each step, and every docker command in full
VZ_LOG=debug vz       # plus docker-wrapper's spans: each CLI call, exit code, output size
```

## Test

Build first (`just build`); the example tests run `target/.../release/vz`, or `$VZ`.
Each runs with a throwaway `HOME` (`/tmp/vz-examples/<example>/home`), so `~` never touches yours,
and starts with an empty `.vz_state/`.

```sh
just test                  # unit tests, no engine needed
just examples              # every example's test.sh, against the built vz; needs docker
just example state         # one of them
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
