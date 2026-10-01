use crate::config::MountMode;

/// What ends a configuration file's name: `dev.vz.yml`. Nothing else is
/// read from a folder.
pub const CONFIG_FILE_SUFFIXES: [&str; 2] = [".vz.yml", ".vz.yaml"];
/// The one short name read as well: a repository's plain `vz.yml`.
pub const SHORT_CONFIG_FILE: &str = "vz.yml";

/// The library: `$XDG_CONFIG_HOME/viz-shell`, or `~/.config/viz-shell`
/// without it. Configurations a repository extends by name.
pub const LIBRARY_DIR: &str = "viz-shell";

/// The library's one template, which a first run writes: its name, its file,
/// and the Dockerfile it builds.
pub const LIBRARY_BASE: &str = "vz-debian-trixie";
pub const LIBRARY_BASE_FILE: &str = "vz-debian-trixie.vz.yml";
pub const LIBRARY_BASE_DOCKERFILE: &str = "vz-debian-trixie.Dockerfile";

/// The configuration `vz` runs without `-c`: the repository's. The name of a
/// scope's one nameless configuration.
pub const DEFAULT_CONFIG: &str = "default";

/// Selects a configuration when `-c` is not given; CI sets it once.
pub const CONFIG_ENV: &str = "VZ_CONFIG";

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

/// Built images are named `vz-<the Dockerfile's folder>:<content hash>`.
pub const BUILT_IMAGE_PREFIX: &str = "vz-";

/// The build arg of a Dockerfile that stacks on the image below it.
pub const BASE_ARG: &str = "BASE";

/// The image name when the Dockerfile's folder has no usable characters.
pub const FALLBACK_IMAGE_NAME: &str = "repo";

/// Hex digits of the content hash kept in a built image's tag.
pub const CONTENT_HASH_LEN: usize = 16;

/// Where the launcher mounts its own binary, and runs it as the entrypoint.
pub const ENTRYPOINT_PATH: &str = "/run/viz-shell/viz-shell";

/// A tmpfs, empty on every start: the entrypoint marks it once the user is
/// set up, and an attach waits for the mark.
pub const SESSION_DIR: &str = "/run/viz-shell/session";
pub const READY_FILE: &str = "/run/viz-shell/session/ready";

/// Containers are named `vz-<index>-<repository>[-<name>]`.
pub const CONTAINER_PREFIX: &str = "vz-";

/// The labels that identify a container: vz finds its containers by these.
pub const REPO_LABEL: &str = "vz.repo";
pub const INDEX_LABEL: &str = "vz.index";
pub const NAME_LABEL: &str = "vz.name";
pub const CONFIG_LABEL: &str = "vz.config";
pub const PERSISTENT_LABEL: &str = "vz.persistent";
/// A hash of the effective configuration it was created with.
pub const CONFIG_HASH_LABEL: &str = "vz.config_hash";
/// The configuration chain it was created with, `<config>@<file>` per step
/// in fold order, and its image chain, tags bottom first; each joined by `,`.
pub const CHAIN_LABEL: &str = "vz.chain";
pub const IMAGE_LABEL: &str = "vz.image";

/// The longest label vz writes, in bytes.
pub const MAX_LABEL_LEN: usize = 4096;

/// The secure floor: every capability dropped but these, which only the
/// container's root processes use. The entrypoint gives folders to the user,
/// joins groups and becomes the user; the init, PID 1, forwards signals to
/// the user's processes, and exits if it cannot. Becoming the user clears
/// them: its processes hold none.
pub const FLOOR_CAPABILITIES: [&str; 4] = ["CHOWN", "SETUID", "SETGID", "KILL"];

/// On the floor, setuid programs such as sudo or su gain nothing.
pub const NO_NEW_PRIVILEGES: &str = "no-new-privileges";

/// On the floor, at most this many processes: a runaway or a fork bomb stops
/// here, not at the host's limit. Builds and agents stay well below it.
pub const FLOOR_PIDS_LIMIT: i64 = 512;

/// On docker's own network, the host answers to this name, as it does in
/// Docker Desktop: a shell reaches a service the host runs.
pub const HOST_ALIAS: &str = "host.docker.internal:host-gateway";

/// Tells the entrypoint that `privileges.sudo` is granted.
pub const SUDO_ENV: &str = "VZ_SUDO";

/// Where the entrypoint grants the user sudo, and where it looks for sudo.
pub const SUDOERS_FILE: &str = "/etc/sudoers.d/viz-shell";
pub const SUDO_BINARIES: [&str; 3] = ["/usr/bin/sudo", "/bin/sudo", "/usr/local/bin/sudo"];

/// The entrypoint starts as root to add the host user, then becomes that user.
pub const CONTAINER_ROOT: &str = "0:0";

pub const PASSWD_FILE: &str = "/etc/passwd";
pub const HOSTS_FILE: &str = "/etc/hosts";

/// The container's hostname: the host's own with `share.host_network`.
pub const HOSTNAME_FILE: &str = "/proc/sys/kernel/hostname";

/// Where a hostname missing from the hosts file resolves, as Debian does.
pub const HOSTNAME_ADDRESS: &str = "127.0.1.1";
pub const GROUP_FILE: &str = "/etc/group";

/// The shell when no command is given and `shell` is unset or missing from
/// the image: the first of these the image has.
pub const SHELLS: [&str; 2] = ["/bin/bash", "/bin/sh"];

/// Tells the entrypoint the configured `shell`.
pub const SHELL_ENV: &str = "VZ_SHELL";

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
pub const PASSTHROUGH_ENV: [&str; 4] = [TERM_ENV, "COLORTERM", "LANG", LOG_ENV];

/// The host's terminal type, copied in. When the image has no description of
/// it, as slim images lack those of newer terminals (ghostty, kitty, wezterm),
/// the shell gets this instead, which every image has.
pub const TERM_ENV: &str = "TERM";
pub const FALLBACK_TERM: &str = "xterm-256color";

/// Where terminal descriptions live, besides `$TERMINFO`, `$TERMINFO_DIRS`
/// and `~/.terminfo`.
pub const TERMINFO_DIRS: [&str; 4] = [
    "/etc/terminfo",
    "/lib/terminfo",
    "/usr/share/terminfo",
    "/usr/lib/terminfo",
];

/// The hooks' commands, each list a JSON array; set only when not empty.
/// `docker exec` inherits them, so `vz enter` reads them too.
pub const HOOKS_CREATE_ENV: &str = "VZ_HOOKS_CREATE";
pub const HOOKS_ATTACH_ENV: &str = "VZ_HOOKS_ATTACH";

/// The repository root, the same path inside as on the host: where hooks run.
pub const REPO_ENV: &str = "VZ_REPO";

/// In the container's writable layer: survives a stop and start, goes with
/// the container. Marks the create hooks as claimed, then as run.
pub const CREATED_DIR: &str = "/var/lib/viz-shell";

/// Where the shell is: the container's name, and the configuration it runs,
/// when one.
pub const CONTAINER_ENV: &str = "VZ_CONTAINER";
pub const CONTAINER_CONFIG_ENV: &str = "VZ_CONTAINER_CONFIG";

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

/// The mode of a mount that names none; a future `defaults:` setting may
/// make it configurable.
pub const DEFAULT_MOUNT_MODE: MountMode = MountMode::Ro;
