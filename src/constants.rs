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

/// The build context when `image.context` is unset: the repository root.
pub const DEFAULT_BUILD_CONTEXT: &str = ".";

/// The engine's CLI, which vz runs for every engine operation.
pub const DOCKER_CLI: &str = "docker";

/// Built images are named `vz-<repository directory>:<content hash>`.
pub const BUILT_IMAGE_PREFIX: &str = "vz-";

/// The image name when the repository directory has no usable characters.
pub const FALLBACK_IMAGE_NAME: &str = "repo";

/// Hex digits of the content hash kept in a built image's tag.
pub const CONTENT_HASH_LEN: usize = 16;

/// Where the launcher mounts its own binary, and runs it as the entrypoint.
pub const ENTRYPOINT_PATH: &str = "/run/vz/vz";

/// The entrypoint starts as root to add the host user, then becomes that user.
pub const CONTAINER_ROOT: &str = "0:0";

pub const PASSWD_FILE: &str = "/etc/passwd";
pub const GROUP_FILE: &str = "/etc/group";

/// The shell when no command is given: the first of these the image has.
pub const SHELLS: [&str; 2] = ["/bin/bash", "/bin/sh"];

/// The host user, carried into the container for the entrypoint.
pub const USER_ENV: &str = "VZ_USER";
pub const UID_ENV: &str = "VZ_UID";
pub const GID_ENV: &str = "VZ_GID";
pub const GROUP_ENV: &str = "VZ_GROUP";
pub const HOME_ENV: &str = "HOME";

/// The build arg carrying the host home; `USER_ENV` to `GROUP_ENV` double as
/// build args. vz passes those a Dockerfile declares, so it can bake the user.
pub const HOME_ARG: &str = "VZ_HOME";

/// Host variables copied into the container when set.
pub const PASSTHROUGH_ENV: [&str; 4] = ["TERM", "COLORTERM", "LANG", LOG_ENV];
