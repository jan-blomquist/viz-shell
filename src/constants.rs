/// The repository configuration file, at the repository root.
pub const REPO_CONFIG_FILE: &str = "vz.yml";

/// Our own variable rather than `RUST_LOG`, which the repository's
/// tools may set for themselves.
pub const LOG_ENV: &str = "VZ_LOG";

/// Warnings from everything, plus vz's own progress lines.
pub const DEFAULT_LOG_FILTER: &str = "warn,vz=info";

/// Added to an image reference without a tag or digest; a pull without
/// one fetches every tag.
pub const DEFAULT_TAG: &str = "latest";

/// The build context when `image.context` is unset: the directory vz runs in.
pub const DEFAULT_BUILD_CONTEXT: &str = ".";

/// The engine's CLI, which vz runs for every engine operation.
pub const DOCKER_CLI: &str = "docker";

/// Built images are named `vz-<repository directory>:<content hash>`.
pub const BUILT_IMAGE_PREFIX: &str = "vz-";

/// The image name when the repository directory has no usable characters.
pub const FALLBACK_IMAGE_NAME: &str = "repo";

/// Hex digits of the content hash kept in a built image's tag.
pub const CONTENT_HASH_LEN: usize = 16;
