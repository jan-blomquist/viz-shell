//! The `docker create` for a session: the repository at the same path as on
//! the host, the host's working directory, the state and host mounts, and the
//! host user, recreated by the entrypoint from the launcher's own binary. And
//! the `docker exec` that attaches to one.

use std::path::{Path, PathBuf};

use docker_wrapper::{DockerCommand, ExecCommand, RunCommand};

use crate::config::EffectiveHooks;
use crate::constants::{
    CONTAINER_ENV, CONTAINER_PROFILE_ENV, CONTAINER_ROOT, ENTRYPOINT_PATH, FLOOR_CAPABILITIES,
    FLOOR_PIDS_LIMIT, HOOKS_ATTACH_ENV, HOOKS_CREATE_ENV, HOST_ALIAS, NO_NEW_PRIVILEGES, REPO_ENV,
    SESSION_DIR, SHELL_ENV, SUDO_ENV,
};
use crate::mounts::HostMount;
use crate::share::DockerSocket;
use crate::state::StateMount;
use crate::user::User;

pub struct Session<'a> {
    /// The container's name, and its hostname.
    pub name: &'a str,
    /// The profile it runs, told to the shell with the name.
    pub profile: Option<&'a str>,
    pub labels: &'a [(String, String)],
    /// Kept after the creating shell exits: the container holds, and every
    /// shell attaches, the first included.
    pub persistent: bool,
    pub image: &'a str,
    pub repo_root: &'a Path,
    pub workdir: &'a Path,
    /// The launcher's own binary; static, so it runs in any image.
    pub vz_binary: &'a Path,
    pub user: &'a User,
    pub state: &'a [StateMount],
    pub mounts: &'a [HostMount],
    /// The host's docker socket, when `share.docker` is on.
    pub docker: Option<&'a DockerSocket>,
    /// Host variables to copy in, already filtered to those that are set.
    pub passthrough: &'a [(String, String)],
    /// The configured environment's names: passed as `--env NAME`, their
    /// values only in the docker CLI's own environment.
    pub env_names: &'a [String],
    /// Empty for the shell. Unused when persistent: the shell attaches.
    pub command: &'a [String],
    /// Whether stdin and stdout are a terminal.
    pub tty: bool,
    /// `privileges.sudo`: docker's default capabilities and sudo, instead of
    /// the secure floor.
    pub sudo: bool,
    /// `share.host_network`: the host's network stack.
    pub host_network: bool,
    /// `shell`: the entrypoint looks it up in the image.
    pub shell: Option<&'a str>,
    /// Run by the entrypoint, and by each `vz enter`.
    pub hooks: &'a EffectiveHooks,
}

impl Session<'_> {
    /// `docker create`'s arguments: those of `docker run`, which takes the
    /// same, under another name.
    pub fn create_args(&self) -> Vec<String> {
        let mut args = self.run_command().build_command_args();
        args[0] = "create".to_owned();
        with_env_names(args, self.env_names)
    }

    fn run_command(&self) -> RunCommand {
        let entrypoint_args: Vec<String> = match self.persistent {
            true => vec!["entrypoint".to_owned(), "--hold".to_owned()],
            false => ["entrypoint", "--"]
                .into_iter()
                .map(str::to_owned)
                .chain(self.command.iter().cloned())
                .collect(),
        };
        let mut run = RunCommand::new(self.image)
            .name(self.name)
            .hostname(self.name)
            .init()
            .user(CONTAINER_ROOT)
            .entrypoint(ENTRYPOINT_PATH)
            .workdir(self.workdir)
            .tmpfs(SESSION_DIR)
            .cmd(entrypoint_args);
        run = self.labels.iter().fold(run, |run, (label, value)| {
            run.label(format!("{label}={value}"))
        });
        run = self
            .binds()
            .iter()
            .fold(run, |run, bind| run.mount(bind.to_mount_arg()));
        if !self.persistent {
            run = run.remove().interactive();
            if self.tty {
                run = run.tty();
            }
        }
        run = match self.host_network {
            true => run.network("host"),
            false => run.add_host(HOST_ALIAS),
        };
        if let Some(shell) = self.shell {
            run = run.env(SHELL_ENV, shell);
        }
        run = match self.sudo {
            true => run.env(SUDO_ENV, "1"),
            false => FLOOR_CAPABILITIES
                .into_iter()
                .fold(run.cap_drop("ALL"), |run, capability| {
                    run.cap_add(capability)
                })
                .security_opt(NO_NEW_PRIVILEGES)
                .pids_limit(FLOOR_PIDS_LIMIT),
        };
        for (name, commands) in [
            (HOOKS_CREATE_ENV, &self.hooks.create),
            (HOOKS_ATTACH_ENV, &self.hooks.attach),
        ] {
            if !commands.is_empty() {
                let json = serde_json::to_string(commands).expect("a list of strings is JSON");
                run = run.env(name, json);
            }
        }
        run = run.env(REPO_ENV, self.repo_root.to_string_lossy());
        run = run.env(CONTAINER_ENV, self.name);
        if let Some(profile) = self.profile {
            run = run.env(CONTAINER_PROFILE_ENV, profile);
        }
        let docker_env = self.docker.map(DockerSocket::env).into_iter().flatten();
        let env: Vec<(String, String)> = self
            .user
            .env()
            .into_iter()
            .chain(docker_env)
            .map(|(name, value)| (name.to_owned(), value))
            .chain(self.passthrough.iter().cloned())
            .collect();
        env.iter()
            .fold(run, |run, (name, value)| run.env(name, value))
    }
}

/// The `docker exec` that attaches a shell, or a command, to a running
/// container: vz's own binary, which waits for the entrypoint's setup, then
/// becomes the user.
pub struct Enter<'a> {
    pub container: &'a str,
    pub workdir: &'a Path,
    pub tty: bool,
    pub passthrough: &'a [(String, String)],
    /// As for `Session`: by name, values in the docker CLI's environment.
    pub env_names: &'a [String],
    /// Empty for the shell.
    pub command: &'a [String],
}

impl Enter<'_> {
    pub fn args(&self) -> Vec<String> {
        let command = [ENTRYPOINT_PATH, "enter", "--"]
            .into_iter()
            .map(str::to_owned)
            .chain(self.command.iter().cloned())
            .collect();
        let mut exec = ExecCommand::new(self.container, command)
            .interactive()
            .workdir(self.workdir);
        if self.tty {
            exec = exec.tty();
        }
        exec = self
            .passthrough
            .iter()
            .fold(exec, |exec, (name, value)| exec.env(name, value));
        with_env_names(exec.build_command_args(), self.env_names)
    }
}

/// docker-wrapper writes every `--env` as `NAME=VALUE`; the configured
/// environment goes in as bare names, right after the subcommand, so values
/// never reach a command line or a log.
fn with_env_names(mut args: Vec<String>, env_names: &[String]) -> Vec<String> {
    let names = env_names
        .iter()
        .flat_map(|name| ["--env".to_owned(), name.clone()]);
    args.splice(1..1, names);
    args
}

impl Session<'_> {
    /// Every bind mount, shallowest destination first, so a deeper mount
    /// always lands on top of one that holds it: the read-write repository
    /// inside a read-only `~/repos`.
    fn binds(&self) -> Vec<Bind<'_>> {
        let mut binds = vec![
            Bind::new(self.vz_binary, Path::new(ENTRYPOINT_PATH), true),
            Bind::new(self.repo_root, self.repo_root, false),
        ];
        binds.extend(
            self.state
                .iter()
                .map(|mount| Bind::new(&mount.source, &mount.target, false)),
        );
        binds.extend(
            self.mounts
                .iter()
                .map(|mount| Bind::new(&mount.source, &mount.target, mount.read_only)),
        );
        binds.extend(
            self.docker
                .map(|socket| Bind::new(&socket.path, &socket.path, false)),
        );
        binds.sort_by_key(|bind| bind.dst.components().count());
        binds
    }
}

struct Bind<'a> {
    src: &'a Path,
    dst: &'a Path,
    read_only: bool,
}

impl<'a> Bind<'a> {
    fn new(src: &'a Path, dst: &'a Path, read_only: bool) -> Self {
        Self {
            src,
            dst,
            read_only,
        }
    }

    fn to_mount_arg(&self) -> String {
        let read_only = if self.read_only { ",readonly" } else { "" };
        format!(
            "type=bind,src={},dst={}{read_only}",
            self.src.display(),
            self.dst.display()
        )
    }
}

/// Inside a vz container, a nested vz asks the host's daemon to mount its
/// own binary by path, so that path must exist on the host too. vz shows
/// host paths at the same path, except its own binary at /run/viz-shell: a
/// binary on any other mount is on the host; one in the image itself, on
/// `/`, or the self-mount is not. `mount_points` is the container's mount
/// table; `None` outside a vz container.
pub fn check_binary_reachable(
    vz_binary: &Path,
    mount_points: Option<&[PathBuf]>,
) -> anyhow::Result<()> {
    let Some(mount_points) = mount_points else {
        return Ok(());
    };
    let self_mount = Path::new(ENTRYPOINT_PATH)
        .parent()
        .unwrap_or(Path::new("/"));
    let covering = mount_points
        .iter()
        .filter(|point| vz_binary.starts_with(point))
        .max_by_key(|point| point.components().count());
    let on_the_host =
        covering.is_some_and(|point| point != Path::new("/") && !point.starts_with(self_mount));
    anyhow::ensure!(
        on_the_host,
        "inside a vz container, run a viz-shell on a path the host has too, such as the \
         repository's target/x86_64-unknown-linux-musl/release/viz-shell: the host's docker \
         mounts {} from the host",
        vz_binary.display()
    );
    Ok(())
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::*;
    use crate::cli::{Action, Cli};
    use crate::config::{ConfigFile, StateKind};

    fn sally() -> User {
        User {
            name: "sally".to_owned(),
            uid: 1000,
            gid: 1000,
            group: "sally".to_owned(),
            home: PathBuf::from("/home/sally"),
        }
    }

    fn args_for(command: &[String], tty: bool) -> Vec<String> {
        args_with(command, tty, false, false)
    }

    fn args_with(command: &[String], tty: bool, sudo: bool, host_network: bool) -> Vec<String> {
        args_configured(command, tty, sudo, host_network, |_| {})
    }

    fn args_configured(
        command: &[String],
        tty: bool,
        sudo: bool,
        host_network: bool,
        configure: impl FnOnce(&mut Session),
    ) -> Vec<String> {
        let user = sally();
        let state = [StateMount {
            source: PathBuf::from(
                "/home/sally/repos/vz/.vz_state/home/sally/.config/opencode/opencode.json",
            ),
            target: PathBuf::from("/home/sally/.config/opencode/opencode.json"),
            kind: StateKind::File,
            init: None,
        }];
        let mounts = [
            HostMount {
                source: PathBuf::from("/home/sally/repos"),
                target: PathBuf::from("/home/sally/repos"),
                read_only: true,
                point_in_state: None,
                file: ConfigFile::Repository,
            },
            HostMount {
                source: PathBuf::from("/home/sally/repos/skills"),
                target: PathBuf::from("/home/sally/.agents/skills"),
                read_only: true,
                point_in_state: None,
                file: ConfigFile::Repository,
            },
        ];
        let docker = DockerSocket {
            path: PathBuf::from("/run/user/1000/docker.sock"),
            group: "docker".to_owned(),
            gid: 969,
        };
        let passthrough = [("TERM".to_owned(), "xterm-256color".to_owned())];
        let env_names = ["GH_TOKEN".to_owned()];
        let labels = [("vz.index".to_owned(), "0".to_owned())];
        let hooks = EffectiveHooks::default();
        let mut session = Session {
            name: "vz-0-vz",
            profile: None,
            labels: &labels,
            persistent: false,
            image: "vz-vz:abc",
            repo_root: Path::new("/home/sally/repos/vz"),
            workdir: Path::new("/home/sally/repos/vz/src"),
            vz_binary: Path::new("/home/sally/.local/bin/viz-shell"),
            user: &user,
            state: &state,
            mounts: &mounts,
            docker: Some(&docker),
            passthrough: &passthrough,
            env_names: &env_names,
            command,
            tty,
            sudo,
            host_network,
            shell: None,
            hooks: &hooks,
        };
        configure(&mut session);
        session.create_args()
    }

    fn has(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|pair| pair == [flag, value])
    }

    #[test]
    fn run_command__any_session__mounts_repo_at_same_path_and_vz_read_only() {
        let args = args_for(&[], true);

        assert!(
            has(
                &args,
                "--mount",
                "type=bind,src=/home/sally/repos/vz,dst=/home/sally/repos/vz"
            ),
            "{args:?}"
        );
        assert!(
            has(
                &args,
                "--mount",
                "type=bind,src=/home/sally/.local/bin/viz-shell,dst=/run/viz-shell/viz-shell,readonly"
            ),
            "{args:?}"
        );
    }

    #[test]
    fn run_command__state__bind_mounts_each_from_the_cache() {
        let args = args_for(&[], true);

        assert!(
            has(
                &args,
                "--mount",
                "type=bind,src=/home/sally/repos/vz/.vz_state/home/sally/.config/opencode/opencode.json,dst=/home/sally/.config/opencode/opencode.json"
            ),
            "{args:?}"
        );
    }

    #[test]
    fn run_command__any_session__starts_the_entrypoint_as_root_in_the_workdir() {
        let args = args_for(&[], true);

        assert!(has(&args, "--user", "0:0"), "{args:?}");
        assert!(
            has(&args, "--entrypoint", "/run/viz-shell/viz-shell"),
            "{args:?}"
        );
        assert!(
            has(&args, "--workdir", "/home/sally/repos/vz/src"),
            "{args:?}"
        );
    }

    #[test]
    fn run_command__any_session__carries_user_and_passthrough_env() {
        let args = args_for(&[], true);

        assert!(has(&args, "--env", "VZ_UID=1000"), "{args:?}");
        assert!(has(&args, "--env", "HOME=/home/sally"), "{args:?}");
        assert!(
            has(&args, "--env", "VZ_REPO=/home/sally/repos/vz"),
            "{args:?}"
        );
        assert!(has(&args, "--env", "TERM=xterm-256color"), "{args:?}");
    }

    #[test]
    fn run_command__command_given__follows_image_after_entrypoint_marker() {
        let command = ["id".to_owned(), "-u".to_owned()];

        let args = args_for(&command, false);

        let tail = &args[args.len() - 5..];
        assert_eq!(tail, ["vz-vz:abc", "entrypoint", "--", "id", "-u"]);
    }

    /// The launcher writes the entrypoint's arguments; the cli in the
    /// container reads them. Both sides must agree, flags and `--` included.
    #[test]
    fn run_command__entrypoint_args__parse_back_to_the_same_command() {
        let cases: [&[&str]; 3] = [&[], &["id", "-u"], &["cargo", "test", "--", "--nocapture"]];
        for command in cases {
            let command: Vec<String> = command.iter().map(|arg| arg.to_string()).collect();
            let args = args_for(&command, false);
            let after_image = args.iter().position(|arg| arg == "vz-vz:abc").unwrap() + 1;

            let cli = Cli::try_parse_from(
                std::iter::once("vz".to_owned()).chain(args[after_image..].iter().cloned()),
            )
            .unwrap();

            let expected = Action::Entrypoint {
                hold: false,
                command: command.clone(),
            };
            assert_eq!(cli.action, Some(expected), "command: {command:?}");
        }
    }

    #[test]
    fn run_args__configured_env__by_name_only() {
        let args = args_for(&[], false);

        assert!(has(&args, "--env", "GH_TOKEN"), "{args:?}");
        assert!(
            !args.iter().any(|arg| arg.starts_with("GH_TOKEN=")),
            "{args:?}"
        );
        assert_eq!(&args[..3], ["create", "--env", "GH_TOKEN"]);
    }

    #[test]
    fn run_command__no_terminal__omits_tty() {
        let args = args_for(&[], false);

        assert!(!args.contains(&"--tty".to_owned()), "{args:?}");
    }

    #[test]
    fn run_command__docker_shared__socket_at_its_path_with_host_and_group() {
        let args = args_for(&[], true);

        assert!(
            has(
                &args,
                "--mount",
                "type=bind,src=/run/user/1000/docker.sock,dst=/run/user/1000/docker.sock"
            ),
            "{args:?}"
        );
        assert!(
            has(
                &args,
                "--env",
                "DOCKER_HOST=unix:///run/user/1000/docker.sock"
            ),
            "{args:?}"
        );
        assert!(has(&args, "--env", "VZ_GROUPS=docker:969"), "{args:?}");
    }

    #[test]
    fn run_args__default__secure_floor() {
        let args = args_for(&[], false);

        assert!(has(&args, "--cap-drop", "ALL"), "{args:?}");
        for capability in ["CHOWN", "SETUID", "SETGID", "KILL"] {
            assert!(
                has(&args, "--cap-add", capability),
                "{capability}: {args:?}"
            );
        }
        assert!(
            has(&args, "--security-opt", "no-new-privileges"),
            "{args:?}"
        );
        assert!(has(&args, "--pids-limit", "512"), "{args:?}");
        assert!(!args.iter().any(|arg| arg == "VZ_SUDO=1"), "{args:?}");
    }

    #[test]
    fn run_args__sudo__docker_defaults_and_the_entrypoint_told() {
        let args = args_with(&[], false, true, false);

        assert!(has(&args, "--env", "VZ_SUDO=1"), "{args:?}");
        let floor = ["--cap-drop", "--cap-add", "--security-opt", "--pids-limit"];
        assert!(
            !args.iter().any(|arg| floor.contains(&arg.as_str())),
            "{args:?}"
        );
    }

    #[test]
    fn run_args__shell__the_entrypoint_told_only_when_set() {
        let fish = args_configured(&[], false, false, false, |session| {
            session.shell = Some("fish")
        });
        let unset = args_with(&[], false, false, false);

        assert!(has(&fish, "--env", "VZ_SHELL=fish"), "{fish:?}");
        assert!(
            !unset.iter().any(|arg| arg.starts_with("VZ_SHELL")),
            "{unset:?}"
        );
    }

    #[test]
    fn run_command__hooks__json_arrays_in_env_only_when_set() {
        // The session borrows it for any lifetime the helper picks.
        let hooks = Box::leak(Box::new(EffectiveHooks {
            create: vec!["npm ci".to_owned(), "echo \"a b\"".to_owned()],
            attach: vec!["git fetch".to_owned()],
        }));
        let set = args_configured(&[], false, false, false, |session| session.hooks = hooks);
        let unset = args_for(&[], false);

        assert!(
            has(
                &set,
                "--env",
                r#"VZ_HOOKS_CREATE=["npm ci","echo \"a b\""]"#
            ),
            "{set:?}"
        );
        assert!(
            has(&set, "--env", r#"VZ_HOOKS_ATTACH=["git fetch"]"#),
            "{set:?}"
        );
        assert!(
            !unset.iter().any(|arg| arg.starts_with("VZ_HOOKS")),
            "{unset:?}"
        );
    }

    #[test]
    fn run_args__host_network__the_hosts_network_else_dockers() {
        let shared = args_with(&[], false, false, true);
        let default = args_with(&[], false, false, false);

        assert!(has(&shared, "--network", "host"), "{shared:?}");
        assert!(!default.iter().any(|arg| arg == "--network"), "{default:?}");
        // On docker's network the host has a name; on its own network, localhost.
        let alias = "host.docker.internal:host-gateway";
        assert!(has(&default, "--add-host", alias), "{default:?}");
        assert!(!shared.iter().any(|arg| arg == "--add-host"), "{shared:?}");
    }

    #[test]
    fn create_args__any_session__named_labeled_with_a_fresh_session_dir() {
        let args = args_for(&[], true);

        assert!(has(&args, "--name", "vz-0-vz"), "{args:?}");
        assert!(has(&args, "--hostname", "vz-0-vz"), "{args:?}");
        assert!(has(&args, "--label", "vz.index=0"), "{args:?}");
        assert!(has(&args, "--tmpfs", "/run/viz-shell/session"), "{args:?}");
    }

    #[test]
    fn create_args__any_session__tells_the_shell_its_container_and_profile() {
        let plain = args_for(&[], false);
        let trusted = args_configured(&[], false, false, false, |session| {
            session.profile = Some("trusted")
        });

        assert!(has(&plain, "--env", "VZ_CONTAINER=vz-0-vz"), "{plain:?}");
        assert!(
            !plain
                .iter()
                .any(|arg| arg.starts_with("VZ_CONTAINER_PROFILE")),
            "{plain:?}"
        );
        assert!(
            has(&trusted, "--env", "VZ_CONTAINER_PROFILE=trusted"),
            "{trusted:?}"
        );
    }

    #[test]
    fn create_args__not_persistent__removed_on_exit_running_the_command() {
        let args = args_for(&["id".to_owned()], true);

        for flag in ["--rm", "--interactive", "--tty"] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}: {args:?}");
        }
        assert_eq!(args[args.len() - 3..], ["entrypoint", "--", "id"]);
    }

    #[test]
    fn create_args__persistent__kept_holding_for_shells_to_attach() {
        let args = args_configured(&["id".to_owned()], true, false, false, |session| {
            session.persistent = true
        });

        for flag in ["--rm", "--interactive", "--tty"] {
            assert!(!args.iter().any(|arg| arg == flag), "{flag}: {args:?}");
        }
        assert_eq!(
            args[args.len() - 3..],
            ["vz-vz:abc", "entrypoint", "--hold"]
        );
    }

    #[test]
    fn enter_args__command__vz_enters_the_container_as_the_user() {
        let passthrough = [("TERM".to_owned(), "xterm".to_owned())];
        let env_names = ["GH_TOKEN".to_owned()];
        let command = ["id".to_owned(), "-u".to_owned()];
        let enter = Enter {
            container: "vz-0-vz",
            workdir: Path::new("/home/sally/repos/vz"),
            tty: true,
            passthrough: &passthrough,
            env_names: &env_names,
            command: &command,
        };

        let args = enter.args();

        assert_eq!(&args[..3], ["exec", "--env", "GH_TOKEN"]);
        assert!(has(&args, "--env", "TERM=xterm"), "{args:?}");
        assert!(has(&args, "--workdir", "/home/sally/repos/vz"), "{args:?}");
        assert!(args.iter().any(|arg| arg == "--tty"), "{args:?}");
        let tail = &args[args.len() - 6..];
        assert_eq!(
            tail,
            [
                "vz-0-vz",
                "/run/viz-shell/viz-shell",
                "enter",
                "--",
                "id",
                "-u"
            ]
        );
    }

    /// The exec's arguments after the binary parse back into `enter`.
    #[test]
    fn enter_args__after_the_binary__parse_as_enter() {
        let command = ["cargo".to_owned(), "test".to_owned(), "--".to_owned()];
        let enter = Enter {
            container: "vz-0-vz",
            workdir: Path::new("/"),
            tty: false,
            passthrough: &[],
            env_names: &[],
            command: &command,
        };
        let args = enter.args();
        let after_binary = args.iter().position(|arg| arg == ENTRYPOINT_PATH).unwrap() + 1;

        let cli = Cli::try_parse_from(
            std::iter::once("vz".to_owned()).chain(args[after_binary..].iter().cloned()),
        )
        .unwrap();

        let expected = Action::Enter {
            command: command.to_vec(),
        };
        assert_eq!(cli.action, Some(expected));
    }

    #[test]
    fn check_binary_reachable__cases() {
        let mounts: Vec<PathBuf> = [
            "/",
            "/proc",
            "/home/sally/repos",
            "/home/sally/repos/vz",
            "/run/viz-shell/viz-shell",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let built = "/home/sally/repos/vz/target/x86_64-unknown-linux-musl/release/viz-shell";
        let cases = [
            // Outside a vz container, anything goes.
            (None, "/run/viz-shell/viz-shell", true),
            // On a same-path mount: the repository, or a sibling under ~/repos.
            (Some(&mounts[..]), built, true),
            (Some(&mounts[..]), "/home/sally/repos/other/viz-shell", true),
            // The self-mount, or the image itself.
            (Some(&mounts[..]), "/run/viz-shell/viz-shell", false),
            (Some(&mounts[..]), "/usr/local/bin/viz-shell", false),
        ];
        for (mounts, binary, reachable) in cases {
            let inside_vz = mounts.is_some();
            let result = check_binary_reachable(Path::new(binary), mounts);

            assert_eq!(result.is_ok(), reachable, "{inside_vz} {binary}");
        }
    }

    #[test]
    fn create_args__mount_with_a_target__host_source_at_its_target() {
        let args = args_for(&[], true);

        assert!(
            has(
                &args,
                "--mount",
                "type=bind,src=/home/sally/repos/skills,dst=/home/sally/.agents/skills,readonly"
            ),
            "{args:?}"
        );
    }

    #[test]
    fn run_command__read_only_parent_of_the_repository__mounted_before_it() {
        let args = args_for(&[], true);

        let position = |mount: &str| args.iter().position(|arg| arg == mount).unwrap();
        let parent = position("type=bind,src=/home/sally/repos,dst=/home/sally/repos,readonly");
        let repository = position("type=bind,src=/home/sally/repos/vz,dst=/home/sally/repos/vz");
        assert!(parent < repository, "{args:?}");
    }
}
