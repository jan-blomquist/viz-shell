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
- [Share](#share)
- [Privileges](#privileges)
- [Environment](#environment)
- [Profiles](#profiles)
- [Global configuration](#global-configuration)
- [Examples](#examples)
- [Logging](#logging)
- [Test](#test)
- [License](#license)

## Build

```sh
just build      # → target/x86_64-unknown-linux-musl/release/viz-shell
just install    # → ~/.local/bin/viz-shell2, and the alias ~/.local/bin/vz2
just install "" # → viz-shell and vz, once the legacy viz-shell no longer holds those names
```

Rust 1.95.0 and the musl target are pinned in `rust-toolchain.toml`. The binary is static,
because `vz` mounts itself into every container it starts: build it anywhere, run it on any Linux host.

## Use

```sh
vz                  # a shell: bash, else sh
vz -- cargo test    # one command instead
vz -c other.yml     # another configuration: -c, --config-file
vz --profile ci     # a profile from vz.yml; or VZ_PROFILE=ci
vz --show-effective-config   # the configuration vz would run with, as vz.yml YAML; runs nothing
vz profiles         # the profiles of the global and the repository configuration
```

`vz` (the alias of `viz-shell`) reads its configuration at the git root, the first of `viz-shell.yml`,
`viz-shell.yaml`, `vz.yml`, `vz.yaml` (it warns about any others), or the `-c` file. It pulls or builds the image if missing, and runs a container
that is removed on exit; `vz` exits with its exit code. Inside:

- the repository is mounted read-write at its host path; the working directory is yours;
- you are you: same user, uid, group and home, so `whoami`, `~` and ssh work as on the host;
- `TERM`, `COLORTERM`, `LANG` and `VZ_LOG` are copied in when set.

`banner: true` prints the viz-shell banner above an interactive shell (never above `vz -- command`):

```
       _              _          _ _
__   _(_)____     ___| |__   ___| | |
\ \ / / |_  /____/ __| '_ \ / _ \ | |
 \ V /| |/ /_____\__ \ | | |  __/ | |
  \_/ |_/___|    |___/_| |_|\___|_|_|

  vz | profile: trusted | sudo: yes | docker: yes | host network: yes
```

The global template turns it on; `banner: false` in a repository or profile turns it off.

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
state_dir: .vz_state                   # optional: the state folder, see below
state:
  - ~/.local/share/opencode            # a folder
  - /var/cache/apt                     # any absolute path
  - { path: ~/.config/opencode/opencode.json, type: file, init: "{}" }   # a file, "{}" the first time
```

| Entry | Inside | Kept at |
|---|---|---|
| `~/.local/share/opencode` | `/home/sally/.local/share/opencode` | `.vz_state/home/sally/.local/share/opencode` |
| `/var/cache/apt` | `/var/cache/apt` | `.vz_state/var/cache/apt` |

- Expanded: `{ path, type: dir | file, init, enabled }`. `init` is a file's content when `vz`
  creates it; never rewritten.
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
  - ~/repos                              # read-only
  - { path: ~/.config/gh, mode: rw }     # read-write
```

- The repository `vz` runs for is always read-write, even inside a read-only mount like `~/repos`:
  deeper mounts land on top.
- A mount must exist on the host; `vz` never creates one.
- A single file mounts too, with two catches: a read-write one breaks tools that save by renaming
  over it ("Device or resource busy"), and a running container keeps seeing the old version when
  the host replaces the file by renaming, as many editors and `git config` do. Folders have neither.
- `- ~/.ssh` gives ssh inside your keys, `config` and `known_hosts`, as on the host. The keys are
  then readable by everything in the container: mount it only where you trust what runs there.
- Paths follow the state rules; a mount may not overlap a state path.

## Share

What of the host the shell shares; nothing unless a layer turns it on.

```yaml
share:
  docker: true         # the host's docker daemon
  host_network: true   # the host's network stack
```

- `docker`: the socket behind the current docker endpoint (it follows `DOCKER_HOST` and
  `docker context use`) is mounted at its own path, `DOCKER_HOST` points at it, and you join its
  group: docker works inside as you, without sudo. The image needs the docker CLI.
- Sharing the daemon gives the shell root-equivalent control of the host: only for trusted repositories.
- `host_network`: `--network host`, the host's network stack, its `localhost` and its ports. Without
  it the shell still reaches the internet, through docker's own network, but not the host's `localhost`.
  It gives no root, but the shell reaches every service the host does, and its ports can clash.
- `false` in a profile turns either off: `share: { docker: false }`.
- `vz` inside `vz` talks to the host's daemon, which mounts host paths: run the `vz` built in the
  repository (`target/…/release/viz-shell`); another is refused.

## Privileges

What the shell may do inside; nothing beyond the secure floor unless a layer grants it.

```yaml
privileges:
  sudo: true      # root through sudo; the image needs sudo
```

- The secure floor, by default: every Linux capability dropped, `no-new-privileges` set. The
  entrypoint keeps `CHOWN`, `SETUID` and `SETGID` just long enough to set you up; once it becomes you,
  the shell holds no capabilities, and setuid programs such as `sudo` or `su` gain nothing.
- `sudo: true`: docker's default capabilities, no `no-new-privileges`, and a password-less sudoers
  line for you. An image without sudo gets a warning, and the shell starts without it.
- The global template grants it in its `trusted` profile; `false` in a profile takes it back.

## Environment

Variables inside the container, from four sources; later wins:

```yaml
env:
  defaults:                     # 1. written here: the lowest level
    RUST_LOG: info
    REPO_ROOT: ${repo}          #    ${repo} and ${home} are substituted
  files:                        # 2. read on the host, in this order; never mounted
    - .env                      #    skipped when missing
    - { path: .env.required, required: true }   # must exist
  passthrough:                  # 3. copied from the host's environment
    - GH_TOKEN
    - "FMP_*"                   #    `*` and `?` globs
```

```sh
vz --env RUST_LOG=debug         # 4. over everything; `--env NAME` copies the host's
vz --show-env                   # every name and where it comes from, never a value
```

- Paths are relative to the `vz.yml`'s folder, `~/…` or absolute. Files use `.env` syntax: `KEY=value`,
  `#` comments, `export`, and quotes around values with spaces.
- `defaults` is a map, keyed by variable name: a profile overrides per name, `null` removes one.
  `files` and `passthrough` are lists: expanded forms `{ path, required, enabled }` and `{ name, enabled }`.
- An env file tracked by git is refused: its values would be in the repository's history.
- Values reach the container by name (`docker run --env NAME`), never on a command line or in a log;
  `--show-effective-config` and `--show-env` never print a value from a file or the host.
  `docker inspect` of the container still shows them, as for any container environment.
- `vz` sets `HOME`, `VZ_*`, `TERM`, `COLORTERM`, `LANG` and, when docker is shared, `DOCKER_HOST` itself;
  the environment cannot change those.

## Profiles

Named layers on top of the root of `vz.yml`, keyed by name, each with the same keys. Choose one with
`vz --profile NAME` or `VZ_PROFILE=NAME`; plain `vz` uses the root alone.

```yaml
mounts:
  - ~/repos

profiles:
  writable:
    mounts: [{ path: ~/repos, mode: rw }]       # the same path: updated in its place
  isolated:
    mounts: [{ path: ~/repos, enabled: false }] # removed
  scratch:
    extends: isolated                           # starts from isolated
    state: [~/scratch]                          # and adds its own
```

Every collection is a list, merged the same way: root, then the `extends` chain, then the profile.

- An entry is a bare path or name for the common case, or expanded for anything else.
- An entry with the same key (a path; a name for passthrough) updates the earlier one in its place;
  a new one comes last. `enabled: false` removes one. A key twice in one list is refused.
- Settings (`image`, `state_dir`, `banner`, `share`, `privileges`) are replaced; `env.defaults` merges per variable name.
- The two maps: `env.defaults`, keyed by variable name, and `profiles`, keyed by profile name.
- Profiles don't nest; `extends` cycles and unknown names are refused, naming the defined profiles.
- `vz --profile NAME --show-effective-config` prints the result: every layer applied, shorthands spelled out.

## Global configuration

`~/.config/viz-shell/global.yml` (or under `$XDG_CONFIG_HOME`) has the same shape as a repository's
configuration, and every repository starts from it. The first `vz` writes it from
[`templates/global.yml`](templates/global.yml) when there is none, and never overwrites it: an untrusted
default with the banner on, and a `trusted` profile with sudo, docker, the host's network, `~/.ssh` and trusted-only
secrets. Edit it freely.

- A repository without a configuration runs from the global one alone.
- Layers, later wins: global root, repository root, then for the chosen profile and each it extends
  (first extended first): its global section, then its repository section.

| `vz` | layers |
|---|---|
| `vz` | global root → repo root |
| `vz --profile trusted` | … → global `trusted` → repo `trusted` |
| `vz --profile ci`, repo `ci: { extends: trusted }` | … → global `trusted` → repo `trusted` → repo `ci` |

- A profile is a mode: each file says what it adds in it. A repository's `trusted:` adds to the global
  `trusted`, and any profile can `extends: trusted`. A chosen profile beats both roots.
- Relative paths belong to their file: `trusted.env` in `global.yml` is `~/.config/viz-shell/trusted.env`.
  Every path is made absolute when read, so a repository removes a global entry however it writes it.
- `vz profiles` lists each profile and the files that define it; `--show-effective-config` and
  `--show-env` name the files read and the layers applied.

Trust: `vz` runs the configuration it is given; it cannot tell a hostile one, which can name any host
file or share the docker daemon. Review a repository's configuration as you would its code. What
protects the host is what reaches the container: only the repository, and what the configuration
shares, mounts or grants; and, unless `privileges.sudo` is granted, the secure floor inside it.

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
| [`profiles`](examples/profiles) | overriding and removing entries, `extends`, `VZ_PROFILE` |
| [`docker`](examples/docker) | the host's docker daemon inside, as you; off in a profile |
| [`env`](examples/env) | every environment source and their order, a profile's overrides, values kept out of sight |
| [`global`](examples/global) | a repository without configuration, repository over global, trusted-only secrets |
| [`privileges`](examples/privileges) | the secure floor by default; sudo in a profile; an image without sudo |
| [`host-network`](examples/host-network) | the host's network in a profile, docker's own by default |

## Logging

Stderr, filtered by `VZ_LOG` (default `warn,viz_shell=info,docker_wrapper=error`):

```sh
VZ_LOG=viz_shell=debug vz    # each step, and every docker command in full
VZ_LOG=debug vz       # plus docker-wrapper's spans: each CLI call, exit code, output size
```

## Test

Build first (`just build`); the example tests run `target/.../release/viz-shell`, or `$VZ`.
Each runs with a throwaway `HOME` (`target/vz-examples/<example>/home`), so `~` never touches yours,
and starts with an empty `.vz_state/`. This repository shares docker, so everything runs inside `vz`
too: `vz -- just build examples`.

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
