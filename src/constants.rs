/// The repository's configuration file at the git root: the first of these
/// present is used; others present are ignored, with a warning.
pub const REPO_CONFIG_FILES: [&str; 4] = ["viz-shell.yml", "viz-shell.yaml", "vz.yml", "vz.yaml"];

/// Selects a profile when `--profile` is not given; CI sets it once.
pub const PROFILE_ENV: &str = "VZ_PROFILE";

/// Our own variable rather than `RUST_LOG`, which the repository's
/// tools may set for themselves.
pub const LOG_ENV: &str = "VZ_LOG";

/// Warnings from everything, plus vz's own progress lines. docker-wrapper
/// only reports errors: vz turns its failures into its own messages, and
/// expected ones, like an image not yet present, would read as warnings.
pub const DEFAULT_LOG_FILTER: &str = "warn,viz_shell=info,docker_wrapper=error";

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
pub const ENTRYPOINT_PATH: &str = "/run/viz-shell/viz-shell";

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

/// Where state is kept unless `state_dir` says otherwise: this folder at the
/// git root. Each state path is stored under it at its container path.
pub const DEFAULT_STATE_DIR: &str = ".vz_state";

/// A state path starting with this is under the home.
pub const HOME_PREFIX: &str = "~/";

/// The container's mount table, read by the entrypoint.
pub const MOUNTINFO_FILE: &str = "/proc/self/mountinfo";

/// Inside, the docker CLI finds the shared socket through this.
pub const DOCKER_HOST_ENV: &str = "DOCKER_HOST";
pub const UNIX_SOCKET_SCHEME: &str = "unix://";

/// Extra groups the entrypoint makes the user join, as `name:gid`, comma-separated.
pub const GROUPS_ENV: &str = "VZ_GROUPS";

/// The docker socket's group name when the host has no name for its gid.
pub const DOCKER_GROUP_NAME: &str = "docker";
