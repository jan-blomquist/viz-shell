<p align="center">
  <img src="assets/vz-logo.png" alt="vz" width="420">
</p>

<h3 align="center">One shell for every repo.</h3>

<p align="center">
  Versatile dev environments, everywhere, exactly as you want them.<br>
  As yourself, in any image, secure by default, and made for coding agents.
</p>

<p align="center">
  <a href="#license"><img alt="License: MIT or Apache-2.0" src="https://img.shields.io/badge/license-MIT%20or%20Apache--2.0-blue"></a>
  <img alt="Rust 1.95+" src="https://img.shields.io/badge/rust-1.95%2B-orange">
  <img alt="Linux" src="https://img.shields.io/badge/platform-linux-lightgrey">
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#why-vz">Why vz</a> ·
  <a href="#guide">Guide</a> ·
  <a href="#examples">Examples</a>
</p>

---

```console
$ whoami
sally
$ vz
sally@vz-0-app:~/repos/app$ whoami
sally
```

Describe a repository's environment in a few lines of YAML. `vz` starts it in a container and drops
you in: as yourself, in your repository, with only what you chose to share. On a laptop, a server or
a CI runner, for you and for the agents working beside you.

> **Status:** young and moving fast. It works day to day, but the configuration may still change
> before 1.0. Feedback and issues are welcome.

## Why vz

- **Your repo, mounted.** `cd` into any git repository and type `vz`. The repository is there,
  read-write, at the same path as on your host.
- **You, inside.** Same user, uid, groups, home and paths. Files you create stay yours; git and ssh
  just work; paths in errors match your editor's.
- **Any image.** Official images work unchanged, or `vz` builds the repository's own Dockerfile.
  Switch branches, switch environments; nothing rebuilds needlessly.
- **Profiles.** Your defaults in one global file, the project's in the repository, modes on top:
  `vz --profile trusted` for sudo and docker, `ci` for the pipeline.
- **Secure by default, made for agents.** No Linux capabilities, no sudo, no docker, no host network,
  no secrets, unless you grant them. Let an agent loose: of your machine, it reaches the repository
  and nothing else.
- **State that stays.** Caches, shell history and agent sessions survive the container, per
  repository, and never clutter your home.
- **Secrets out of sight.** Env files are read on the host and pass by name: never on a command line,
  never in a log.
- **Sessions.** Name a container, keep it, attach from any terminal: `vz new api`, `vz at api`.
- **One static binary.** No daemon, no runtime, no editor plugin. Just Docker.

## Quick start

Releases, with a one-line install script, are on the way. Until then, build from source (needs
[Rust](https://rustup.rs) and [just](https://just.systems)):

```sh
git clone https://github.com/jan-blomquist/viz-shell && cd viz-shell
just build && just install         # ~/.local/bin/viz-shell, and its alias vz
```

In any git repository, name an image, then run `vz`:

```yaml
# viz-shell.yml
image: rust:1.98.1-slim-trixie
```

The first run also writes `~/.config/viz-shell/viz-shell.global.yml`: your defaults for every
repository, with a `trusted` profile that grants sudo, docker and the host's network when you ask for
it, and the base image's Dockerfile beside it.

A fuller configuration:

```yaml
image:
  dockerfile: Dockerfile               # ARG BASE: stacks on the global configuration's base
shell: fish
state:                                 # survives the container, kept in .vz_state/
  - /usr/local/cargo/registry
  - ~/.local/share/opencode
mounts:
  - ~/repos                            # read-only; this repository stays read-write
env:
  files: [.env]                        # read on the host, never mounted
  passthrough: [GH_TOKEN]
profiles:
  trusted:                             # vz --profile trusted
    privileges: { sudo: true }
    share: { docker: true }
```

## How vz differs

- **Not an editor plugin.** Dev Containers center on the editor; vz is shell-first. Use it with any
  editor, or none, over ssh, on a server, in CI.
- **Not a package manager.** Nix and Devbox assemble packages; vz runs whatever image you give it,
  and builds your Dockerfile when you give it one.
- **Not host integration.** Distrobox and Toolbox deliberately share your home and your host; vz
  isolates by default and shares only what the configuration declares.

## Guide

- [Use](#use)
- [Whose file is it](#whose-file-is-it)
- [How configuration stacks](#how-configuration-stacks)
- [Sessions](#sessions)
- [Images](#images)
- [Your user in the image](#your-user-in-the-image)
- [State](#state)
- [Mounts](#mounts)
- [Share](#share)
- [Privileges](#privileges)
- [Environment](#environment)
- [Hooks](#hooks)
- [Profiles](#profiles)
- [Global configuration](#global-configuration)
- [Local configuration](#local-configuration)
- [Examples](#examples)
- [Logging](#logging)
- [Development](#development)
- [License](#license)

## Use

```sh
vz                  # a shell: `shell`, else bash, else sh
vz -- cargo test    # one command instead
vz -c other.yml     # another configuration: -c, --config-file
vz --profile ci     # the default plus the profile ci; or VZ_PROFILE=ci
vz --show-effective-config   # the configuration vz would run with, as vz.yml YAML; runs nothing
vz profiles         # the profiles of the global, repository and local configuration
vz new api          # a container named api; see Sessions
vz attach 0         # a shell in container 0 of this repository; or `vz at api`
vz ls               # this repository's containers; --all for every repository's
vz kill 0 api       # removes containers; --all for all of this repository's
```

`vz` (the alias of `viz-shell`) reads its configuration at the git root, the first of `viz-shell.yml`,
`viz-shell.yaml`, `vz.yml`, `vz.yaml` (it warns about any others), or the `-c` file, with the
[global](#global-configuration) and your [local](#local-configuration) configuration. It pulls or
builds the image if missing, and runs a container, removed on exit unless `persistent`, with its
[hooks](#hooks): `create` once, `attach` before every shell or command. `vz` exits with the shell's,
or the command's, exit code. Inside:

- the repository is mounted read-write at its host path; the working directory is yours;
- you are you: same user, uid, group and home, so `whoami`, `~` and ssh work as on the host;
- `TERM`, `COLORTERM`, `LANG` and `VZ_LOG` are copied in when set. A `TERM` the image has no
  description for, as slim images lack those of newer terminals like ghostty, kitty or wezterm,
  becomes `xterm-256color`, so tmux, less and htop still work;
- `VZ_CONTAINER` names the container, `VZ_CONTAINER_PROFILE` its profile, when one, and `VZ_REPO`
  the repository root: for prompts, scripts and agents that want to know where they run.

`banner: true` prints a banner above an interactive shell (never above `vz -- command`), in the
manner of fastfetch: what the shell is about to be. Colored on a terminal, unless `NO_COLOR` is set.
The environment shows as a count, never names or values.

```
       _              _          _ _
__   _(_)____     ___| |__   ___| | |
\ \ / / |_  /____/ __| '_ \ / _ \ | |
 \ V /| |/ /_____\__ \ | | |  __/ | |
  \_/ |_/___|    |___/_| |_|\___|_|_|

sally@vz-0-app
--------------
Version: 0.1.0
Session: new, ephemeral
Repo: ~/repos/app
Branch: main
Config: viz-shell.global.yml, viz-shell.yml, viz-shell.local.yml
Profile: trusted
Image: vz-app:3f9c2a1b7d4e8f60 (on vz-viz-shell:9a1c0d2e5b7f3a41)
Shell: fish
Sudo: yes
Docker: /run/user/1000/docker.sock
Network: host
Mounts: 4 (2 viz-shell.global.yml, 1 viz-shell.yml, 1 viz-shell.local.yml)
State: 2 paths in ~/repos/app/.vz_state
Env: 3 variables
Hooks: 2 create, 1 attach
```

The default global configuration turns it on; `banner: false` in a repository or profile turns it off.

`shell` picks the interactive shell: a name on the image's `PATH`, or an absolute path. It is also
`$SHELL` and your login shell inside. An image without it gives a warning, then bash, else sh, so a
global `shell: fish` doesn't break images without fish. Keep fish's configuration and history per
repository, apart from the host's, as state:

```yaml
shell: fish
state:
  - ~/.config/fish
  - ~/.local/share/fish
```

Unknown keys in `vz.yml` are refused, naming the line.

## Whose file is it

| File | Owner | Checked in | Says |
|---|---|---|---|
| `viz-shell.yml` | the repository | yes | the portable dev environment: image, state, shell, hooks, share, privileges, `env` with its `.env` files and passthrough names |
| `viz-shell.global.yml` | you, everywhere | no | your tools, mounts, credentials, banner, shell preference |
| `viz-shell.local.yml` | you, in this repository | no | mounts and values only this checkout needs |

The portability test for the repository file: no host path outside the repository, except `state`,
which lives inside it. A random user cloning the repository and running `vz` gets the environment;
what they bring is theirs.

## How configuration stacks

Two axes. **Owner**: whose file, `global`, `repository`, `local`; always applied, in that order,
never declared. **Profile**: what was asked for; `default`, a file's top-level keys, always applies,
and `--profile gpu` adds `gpu` and what it `extends`, base-most first.

```
              global   repository   local
default         ●         ●          ●     always
trusted         ●         ●          ·     --profile trusted, or extended by gpu
gpu             ·         ●          ●     --profile gpu
```

- One fold over the grid, profiles outer, owners inner: `default` (global, repository, local), then
  each profile of the chain the same way. A cell with no file, or no section in it, is skipped.
- One merge rule: later wins; keyed lists merge by key. More specific wins; among versions of the
  same thing, the later owner wins.
- Owners are ownership, not inheritance: the repository file cannot opt out of your files, which keeps
  it portable. `extends` is explicit: a choice among profiles.
- `image:` follows the same order, each one replacing the one before, or stacking on it when its
  Dockerfile declares `ARG BASE` ([Images](#images)).
- The image belongs to the repository; the session belongs to the user. No owner can change a
  repository's image unless its Dockerfile declares `ARG BASE`, and no build arg reaches a Dockerfile
  that does not declare it: a Dockerfile with a pinned `FROM` and no `ARG BASE` builds the same for
  everyone. Mounts, env, state, hooks and the shell are the session: yours, on your machine.
- The global file is a convenience, not a requirement: a repository file alone runs `vz`.
- `--show-effective-config` prints the grid as applied and each `image:` with its cell;
  `vz profiles` lists which owners define each profile, `default` first. `default` is no name for a
  profile.

## Sessions

Every container is named `vz-<index>-<repository>`, its hostname too, so your prompt says which one
you are in. The index is the lowest free one of the repository's containers; `vz new api` adds a
name: `vz-1-app-api`. Labels (`vz.repo`, `vz.index`, `vz.name`, `vz.profile`, …) identify them:
`vz ls`, `vz attach` and `vz kill` look containers up by label, by index or name.

```yaml
persistent: true    # the container outlives the shell that created it; `vz kill` removes it
attach: true        # a plain `vz` joins this repository's container of the same profile
```

| `persistent` | `attach` | `vz` | the creating shell exits | the next `vz` |
|---|---|---|---|---|
| false | false | a new container | it is removed | another new one |
| true | false | a new container | it is kept | another new one |
| true | true | a new container, or joins the kept one | it is kept | joins it |
| false | true | a new container, or joins the running one | it is removed, with attached shells | joins it |

- `vz attach [INDEX|NAME] [-- COMMAND]` (or `vz at`) runs a shell, or the command, in a container
  of this repository, as you; without a target, in the only running one. A stopped persistent
  container is started. `attach: true` joins unnamed containers only; a named one is attached by name.
- A container is entered only with its own profile: `vz attach 0` to a container started with
  `--profile trusted` is refused, naming `vz --profile trusted attach 0`. A plain `vz` never lands in
  a trusted container.
- A container keeps the configuration it was created with; attaching after a change warns, and
  `vz kill` then `vz` applies it.
- Attaching is `docker exec` of vz's own binary: it waits for the entrypoint to finish setting you up,
  then becomes you, as the entrypoint does.

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

- A built image is tagged `vz-<Dockerfile's folder>:<hash of Dockerfile + args>` and builds only
  when missing. One Dockerfile used by many repositories, say from the global configuration, is one image.
  Editing the Dockerfile or args rebuilds; editing a copied file does not — remove the image to force it.
- Builds run `docker build`, via [docker-wrapper](https://github.com/joshrotenberg/docker-wrapper),
  so `.dockerignore` applies.
- This repository's `Dockerfile`: the Rust toolchain, fish as its shell, and `vz` built from the
  checkout, stacked on the base (below); alone, on pinned Debian, with what the base adds.

**Stacking.** A Dockerfile that declares `ARG BASE` (`ARG BASE=<default>`, then `FROM ${BASE}`, as
below) is built on the image the earlier cells resolved to: vz pulls or builds that one first, then
passes `--build-arg BASE=<its tag>`. Images follow the [fold order](#how-configuration-stacks), so a
later cell's Dockerfile lands on top. The base's tag joins
the hash: a new base rebuilds what stacks on it. With no earlier image, the default applies. A
Dockerfile without `ARG BASE`, `BASE` set in `args`, or an image reference replaces.
`--show-effective-config` lists the chain, each image marked `stacks` or `replaces`; the banner shows
`Image: vz-app:3f9c2a1b (on vz-tools:9a1c0d2e, debian:stable-slim)`.

**What an image needs.** Official images like `debian`, `alpine`, `rust`, `node` or `python` already
meet the contract. For your own images:

- a shell: `bash`, else `sh`, or the one `shell` names;
- tools under `/usr/local` or `/opt`, not in a home, so the image serves every user and `state`
  mounts in your home cannot shadow them;
- no reliance on an `ENTRYPOINT` (vz runs its own) or on a baked user (vz adds you);
- `sudo`, if a profile grants `privileges.sudo`.

**Base image.** The first run writes `viz-shell.base.Dockerfile` next to the global configuration
([`templates/viz-shell.base.Dockerfile`](templates/viz-shell.base.Dockerfile), embedded in the
binary), and the global configuration's `image:` builds it, locally, on first use, as
`vz-viz-shell:<hash>`. It holds what vz's features need: sudo, the docker CLI, locales,
ca-certificates; nothing else. Edit it, or replace it in the global configuration with a Dockerfile
of your own or an image reference.

**How the image is built.** apt when Debian's version will do. Otherwise the vendor's release,
downloaded from its URL and verified by sha256, or, for a static binary whose vendor publishes an
image as the way to get it, `COPY --from` that image. Every `FROM` is pinned by digest and every
download checksummed.

A repository stacks on it and adds what it needs:

```dockerfile
# The image this one starts from: the global configuration's, else the default.
ARG BASE=debian:trixie-20260918-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
FROM ${BASE}
RUN apt-get update \
 && apt-get install -y --no-install-recommends git fish \
 && rm -rf /var/lib/apt/lists/*
```

An opinionated everyday image, with agents, shells and tools, is a repository of your own, built the
same way; the global configuration's `image: { dockerfile: ... }` can point at a personal Dockerfile
next to `viz-shell.global.yml`, for repositories without their own.

Pin a version, never a moving tag: a new base is then an edit to the `FROM`, which changes the
image's hash, so the repository rebuilds on that branch, and only there.

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

Host paths shown inside, reusing the host's own files: at the same path, unless a target names
another. Written as docker's `-v`: `path[:target][:ro|rw]`. Mounts are read-only unless `:rw`.
The default is read-only, `vz`'s secure-by-default posture; a mount says `:rw` to be writable.

```yaml
mounts:
  - ~/.config/gh:rw                                     # read-write
  - ~/repos                                             # read-only
  - ~/repos/skills:~/.agents/skills                     # elsewhere inside
  - ~/repos/skills:~/.config/opencode/skills            # one source, several targets
```

The map form says the same, key by key, and adds `enabled: false`, which removes an entry an
earlier layer added:

```yaml
mounts:
  - { path: ~/.config/gh, mode: rw }
  - { path: ~/repos/skills, target: ~/.agents/skills, enabled: false }
```

- Each path starts with `~/` or `/`; anything else, a mode other than `ro` or `rw`, or an empty
  field is refused, quoting the entry.
- The repository `vz` runs for is always read-write, even inside a read-only mount like `~/repos`:
  deeper mounts land on top. A mount that lands on the repository itself is skipped, so one repository
  can be read-write for every session, its own included: `- ~/repos/notes:rw`.
  `VZ_LOG=viz_shell=debug` shows the skip.
- A mount must exist on the host; `vz` never creates one.
- A single file mounts too, with two catches: a read-write one breaks tools that save by renaming
  over it ("Device or resource busy"), and a running container keeps seeing the old version when
  the host replaces the file by renaming, as many editors and `git config` do. Folders have neither.
- `- ~/.ssh` gives ssh inside your keys, `config` and `known_hosts`, as on the host. The keys are
  then readable by everything in the container: mount it only where you trust what runs there.
- Mounts are keyed by their target, where they land, else their path: a profile updates or removes
  one by it, in either form, and one source may land in several places.
- A mount may land inside a state folder, such as a library inside a tool's persisted config: `vz`
  creates its mount point in the state folder, as you. It may not hold a state path, sit on one, or
  lie inside a state file.
- Paths follow the state rules.

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
  it the shell still reaches the internet, through docker's own network, and the host answers to
  `host.docker.internal`, as in Docker Desktop, but not on its `localhost`.
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
  container's root processes keep `CHOWN`, `SETUID`, `SETGID` and `KILL`: the entrypoint to set you
  up, the init to pass signals on to your processes. Once the entrypoint becomes you, the shell holds
  no capabilities, and setuid programs such as `sudo` or `su` gain nothing. At most 512 processes
  run, so a runaway or a fork bomb stops there, not at the host's limit.
- `sudo: true`: docker's default capabilities, no `no-new-privileges`, no process limit of its own,
  and a password-less sudoers line for you. An image without sudo gets a warning, and the shell starts without it.
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
- An env file tracked by git is loaded with a warning: its values are in the repository's history.
- Values reach the container by name (`docker create --env NAME`, and `docker exec` when attaching),
  never on a command line or in a log;
  `--show-effective-config` and `--show-env` never print a value from a file or the host.
  `docker inspect` of the container still shows them, as for any container environment.
- `vz` sets `HOME`, `VZ_*`, `TERM`, `COLORTERM`, `LANG` and, when docker is shared, `DOCKER_HOST` itself;
  the environment cannot change those.

## Hooks

Commands run inside the container before you enter it:

```yaml
hooks:
  create:                      # once per container, on its first entry
    - npm ci
    - { run: "fish -c 'set -U fish_greeting'", enabled: false }
  attach:                      # before every entry: each shell, and `vz -- command`
    - git fetch --quiet
```

- Run as you, in the repository root (`$VZ_REPO`), through `sh -c`, with the session's environment;
  each list in order. Output goes to the terminal of the entry that runs them.
- `create` runs once per container, on its first entry; a stop and start doesn't run it again. An
  ephemeral container runs it on every `vz`: keep hooks idempotent and fast.
- `attach` runs before every entry: the first shell, each `vz attach`, and `vz -- command`.
- A failing hook fails the entry, naming it and its exit status. A failed `create` runs again on the
  next entry.
- Entries arriving while `create` runs wait for it.
- Keyed by the command: a profile adds one, replaces one by the same command, or removes one with
  `enabled: false`. An empty command is refused.
- Another shell's syntax goes through it: `fish -c '...'`.

To seed a config file once, a `state` file with `init:` needs no hook.

## Profiles

Named layers on top of the `default` (a file's top-level keys), keyed by name, each with the same
keys. Choose one with `vz --profile NAME` or `VZ_PROFILE=NAME`; plain `vz` uses the default alone.

```yaml
mounts:
  - ~/repos

profiles:
  writable:
    mounts: [~/repos:rw]                        # the same path: updated in its place, read-write
  isolated:
    mounts: [{ path: ~/repos, enabled: false }] # removed
  scratch:
    extends: isolated                           # starts from isolated
    state: [~/scratch]                          # and adds its own
```

Order and merge rule: [How configuration stacks](#how-configuration-stacks). Every collection is a list:

- An entry is a bare path or name for the common case, or expanded for anything else.
- An entry with the same key (a path; a name for passthrough; the command for hooks) updates the
  earlier one in its place; a new one comes last. `enabled: false` removes one. A key twice in one
  list is refused.
- Settings (`image`, `state_dir`, `banner`, `shell`, `persistent`, `attach`) are replaced; `share` and
  `privileges` per key, `env.defaults` per variable name. An `image:` Dockerfile with `ARG BASE`
  stacks instead: see [Images](#images).
- The two maps: `env.defaults`, keyed by variable name, and `profiles`, keyed by profile name.
- Profiles don't nest; `extends` cycles and unknown names are refused, naming the defined profiles.
- `vz --profile NAME --show-effective-config` prints the result: the cells applied, shorthands spelled out.

## Global configuration

`~/.config/viz-shell/viz-shell.global.yml` (or under `$XDG_CONFIG_HOME`) has the same shape as a
repository's configuration, and every repository starts from it. The first `vz` writes it from its
built-in default ([`templates/viz-shell.global.yml`](templates/viz-shell.global.yml), embedded in
the binary) when there is none, and never overwrites it: the [base image](#images) built from
`viz-shell.base.Dockerfile` beside it, the banner on, and a `trusted` profile with sudo, docker, the
host's network, `~/.ssh` and trusted-only secrets. Edit it freely.

- The first of `viz-shell.global.yml`, `viz-shell.global.yaml`, `vz.global.yml`, `vz.global.yaml`.
  A `global.yml`, the former name, is still read when none of them exists.

- A repository without a configuration runs from the global one alone.
- Order and merge rule: [How configuration stacks](#how-configuration-stacks).
- A profile is a mode: each file says what it adds in it. A repository's `trusted:` adds to the global
  `trusted`, and any profile can `extends: trusted`.
- Relative paths belong to their file: `trusted.env` in `viz-shell.global.yml` is
  `~/.config/viz-shell/trusted.env`.
  Every path is made absolute when read, so a repository removes a global entry however it writes it.
- `vz profiles` lists each profile and the owners that define it; `--show-effective-config` and
  `--show-env` name the files read and the cells applied.

Trust: `vz` runs the configuration it is given; it cannot tell a hostile one, which can name any host
file or share the docker daemon. Review a repository's configuration as you would its code. What
protects the host is what reaches the container: only the repository, and what the configuration
shares, mounts or grants; and, unless `privileges.sudo` is granted, the secure floor inside it.

## Local configuration

Your own overlay of the repository's configuration, for this checkout only: a mount of a sibling
repository, a value only your machine needs. Same shape as `vz.yml`.

```yaml
# viz-shell.local.yml
mounts:
  - ~/repos/shared-lib:rw     # the repository mounts it read-only; here, writable
env:
  defaults: { API_URL: http://localhost:8081 }
```

- At the git root, next to the repository's file: the first of `viz-shell.local.yml`,
  `viz-shell.local.yaml`, `vz.local.yml`, `vz.local.yaml`. With `-c foo.yml`, `foo.local.yml` next to it.
- Order: global, repository, local; later wins. Profiles from all three merge by name: a local
  `trusted:` adds to the repository's and the global one.
- Meant to be ignored by git: add `*.local.yml` to `.gitignore`, or to your global gitignore. `vz`
  warns when it is tracked.
- It may define its own profiles, selected with `--profile` like any other, and its own `image:`:
  a personal Dockerfile with `ARG BASE` stacks on the repository's image.
- A local file alone is no configuration: it needs a repository or a global one.
- `--show-effective-config` names it and shows its cells: `default: …, local`, `<profile>: …, local`.

## Examples

Recipes in [`examples/`](examples), each with a `test.sh` that runs it and checks the result.
Copy a folder's `vz.yml` and `Dockerfile` to your repository root, or try one in place with `-c`.

| Recipe | Shows |
|---|---|
| [`pull-image`](examples/pull-image) | the smallest `vz.yml` |
| [`build-dockerfile`](examples/build-dockerfile) | building from a Dockerfile, with args |
| [`image-stack`](examples/image-stack) | `ARG BASE` stacking on a built image and on a reference, replacing Dockerfiles, images named by their folder |
| [`baked-user`](examples/baked-user) | your user baked into the image, installing into your home |
| [`state`](examples/state) | folders, a file with `init`, absolute paths |
| [`mounts`](examples/mounts) | the `path[:target][:ro\|rw]` string form: read-only `~/repos`, a read-write config folder, a single file, one source at several targets, a mount inside state, a mount on the repository skipped |
| [`profiles`](examples/profiles) | overriding and removing entries, `extends`, `VZ_PROFILE` |
| [`docker`](examples/docker) | the host's docker daemon inside, as you; off in a profile |
| [`env`](examples/env) | every environment source and their order, a profile's overrides, values kept out of sight, the `TERM` fallback |
| [`global`](examples/global) | a repository without configuration, repository over global, trusted-only secrets, the former name `global.yml` |
| [`local`](examples/local) | a local overlay over the repository's file: a value, a mount's mode, an added mount, a profile, a profile of its own; the warning when it is tracked |
| [`privileges`](examples/privileges) | the secure floor by default, its process limit; sudo in a profile; an image without sudo |
| [`host-network`](examples/host-network) | the host's network in a profile, docker's own by default, `host.docker.internal` |
| [`shell`](examples/shell) | fish as the shell, its configuration as state; a missing shell's fallback |
| [`sessions`](examples/sessions) | named containers, `VZ_CONTAINER`, persistent ones, attach by index, name or `attach: true`, kill |
| [`hooks`](examples/hooks) | `create` once per container, across a stop and start, and `attach` per entry, in order, in the repository root; one removed in a profile; a failing hook |

## Logging

Stderr, filtered by `VZ_LOG` (default `warn,viz_shell=info,docker_wrapper=error`):

```sh
VZ_LOG=viz_shell=debug vz    # each step, and every docker command in full
VZ_LOG=debug vz       # plus docker-wrapper's spans: each CLI call, exit code, output size
```

## Development

```sh
just build                # → target/x86_64-unknown-linux-musl/release/viz-shell
just install              # → ~/.local/bin/viz-shell, and the alias ~/.local/bin/vz
```

Rust 1.98.1 and the musl target are pinned in `rust-toolchain.toml`. The binary is static,
because `vz` mounts itself into every container it starts: build it anywhere, run it on any Linux host.

Build first (`just build`); the example tests run `target/.../release/viz-shell`, or `$VZ`.
Each runs with a throwaway `HOME` (`target/vz-examples/<example>/home`), so `~` never touches yours,
and starts with an empty `.vz_state/`. This repository shares docker, so everything runs inside `vz`
too: `vz -- just build examples`.

```sh
just test                  # unit tests, no engine needed
just examples              # every example's test.sh, against the built vz; needs docker
just example state         # one of them
```

Issues and pull requests are welcome. Run `just test` and `just examples` before sending one.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
