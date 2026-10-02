# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/jan-blomquist/viz-shell/releases/tag/v0.1.0) - 2026-10-02

### Added

- configurations by name — `*.vz.yml` files, explicit single extends, one chain ([#21](https://github.com/jan-blomquist/viz-shell/pull/21))
- image stacking, local overlay  ([#20](https://github.com/jan-blomquist/viz-shell/pull/20))
- hooks — create once per container, attach before every entry ([#18](https://github.com/jan-blomquist/viz-shell/pull/18))
- mount string form path[:target][:ro|rw], read-write default ([#16](https://github.com/jan-blomquist/viz-shell/pull/16))
- legacy migration features — mount targets, floor parity, TERM fallback ([#15](https://github.com/jan-blomquist/viz-shell/pull/15))
- base images, Rust 1.98.1, README and logo ([#14](https://github.com/jan-blomquist/viz-shell/pull/14))
- sessions — named containers, attach, persistent ([#13](https://github.com/jan-blomquist/viz-shell/pull/13))
- shell setting, fastfetch-style banner ([#12](https://github.com/jan-blomquist/viz-shell/pull/12))
- privileges, secure floor, host network, banner ([#10](https://github.com/jan-blomquist/viz-shell/pull/10))
- viz-shell binary, global configuration, vz profiles ([#9](https://github.com/jan-blomquist/viz-shell/pull/9))
- environment variables; lists for every collection ([#8](https://github.com/jan-blomquist/viz-shell/pull/8))
- ssh inside via a read-only ~/.ssh mount ([#7](https://github.com/jan-blomquist/viz-shell/pull/7))
- share the host's docker daemon ([#6](https://github.com/jan-blomquist/viz-shell/pull/6))
- profiles, map-based config, --show-effective-config ([#5](https://github.com/jan-blomquist/viz-shell/pull/5))
- shell as the host user, with the repo mounted at its host path ([#3](https://github.com/jan-blomquist/viz-shell/pull/3))
- build repo image from Dockerfile; drive docker via CLI ([#2](https://github.com/jan-blomquist/viz-shell/pull/2))

### Fixed

- mounts read-only by default, behind DEFAULT_MOUNT_MODE ([#19](https://github.com/jan-blomquist/viz-shell/pull/19))

### Other

- releases — release-plz, crates.io, the static tarball ([#23](https://github.com/jan-blomquist/viz-shell/pull/23))
- Feat/ci gate ([#22](https://github.com/jan-blomquist/viz-shell/pull/22))
- stacked banner with terse facts, global template as a file ([#17](https://github.com/jan-blomquist/viz-shell/pull/17))
- Feat/add state and mounts ([#4](https://github.com/jan-blomquist/viz-shell/pull/4))
- - single crate, `vz` binary with musl compilation ([#1](https://github.com/jan-blomquist/viz-shell/pull/1))
- Initial commit
