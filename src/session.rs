//! The `docker create` for a session: the repository at the same path as on
//! the host, the host's working directory, the state and host mounts, and the
//! host user, recreated by the entrypoint from the launcher's own binary. And
//! the `docker exec` that attaches to one.

use std::path::{Path, PathBuf};

use docker_wrapper::{DockerCommand, ExecCommand, RunCommand};

use crate::config::EffectiveHooks;
use crate::constants::{
    CONTAINER_CONFIG_ENV, CONTAINER_ENV, CONTAINER_ROOT, ENTRYPOINT_PATH, FLOOR_CAPABILITIES,
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
    /// The configuration it runs, told to the shell with the name.
    pub config: Option<&'a str>,
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
        if let Some(config) = self.config {
            run = run.env(CONTAINER_CONFIG_ENV, config);
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
    use crate::config::StateKind;

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
                file: PathBuf::from("/home/sally/repos/vz/default.vz.yml"),
            },
            HostMount {
                source: PathBuf::from("/home/sally/repos/skills"),
                target: PathBuf::from("/home/sally/.agents/skills"),
                read_only: true,
                point_in_state: None,
                file: PathBuf::from("/home/sally/repos/vz/default.vz.yml"),
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
            config: None,
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

    /// Whether any argument is `arg`: a lone flag such as `--tty`.
    fn has_arg(args: &[String], arg: &str) -> bool {
        args.iter().any(|each| each == arg)
    }

    /// Whether any argument starts with `prefix`: a variable set at all.
    fn has_prefix(args: &[String], prefix: &str) -> bool {
        args.iter().any(|arg| arg.starts_with(prefix))
    }

    /// The arguments after the first `marker`, with `vz` before them: what
    /// the cli inside the container parses.
    fn after(args: &[String], marker: &str) -> Vec<String> {
        let position = args.iter().position(|arg| arg == marker).unwrap();
        std::iter::once("vz".to_owned())
            .chain(args[position + 1..].iter().cloned())
            .collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn run_command__any_session__mounts_repo_at_same_path_and_vz_read_only() {
        let args = args_for(&[], true);

        let expected = [
            "type=bind,src=/home/sally/repos/vz,dst=/home/sally/repos/vz",
            "type=bind,src=/home/sally/.local/bin/viz-shell,dst=/run/viz-shell/viz-shell,readonly",
        ];
        for mount in expected {
            assert!(has(&args, "--mount", mount), "{mount}: {args:?}");
        }
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

        let expected = [
            ("--user", "0:0"),
            ("--entrypoint", "/run/viz-shell/viz-shell"),
            ("--workdir", "/home/sally/repos/vz/src"),
        ];
        for (flag, value) in expected {
            assert!(has(&args, flag, value), "{flag} {value}: {args:?}");
        }
    }

    #[test]
    fn run_command__any_session__carries_user_and_passthrough_env() {
        let args = args_for(&[], true);

        let expected = [
            "VZ_UID=1000",
            "HOME=/home/sally",
            "VZ_REPO=/home/sally/repos/vz",
            "TERM=xterm-256color",
        ];
        for env in expected {
            assert!(has(&args, "--env", env), "{env}: {args:?}");
        }
    }

    #[test]
    fn run_command__command_given__follows_image_after_entrypoint_marker() {
        let command = strings(&["id", "-u"]);

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
            let command = strings(command);
            let args = args_for(&command, false);

            let cli = Cli::try_parse_from(after(&args, "vz-vz:abc")).unwrap();

            let expected = Action::Entrypoint {
                hold: false,
                command: command.clone(),
            };
            assert_eq!(cli.action, Some(expected), "command: {command:?}");
        }
    }

    #[test]
    fn run_args__configured_env__passed_bare_right_after_the_subcommand() {
        let args = args_for(&[], false);

        assert_eq!(&args[..3], ["create", "--env", "GH_TOKEN"]);
    }

    #[test]
    fn run_args__configured_env__its_value_never_on_the_command_line() {
        let args = args_for(&[], false);

        assert!(!has_prefix(&args, "GH_TOKEN="), "{args:?}");
    }

    #[test]
    fn run_command__no_terminal__omits_tty() {
        let args = args_for(&[], false);

        assert!(!has_arg(&args, "--tty"), "{args:?}");
    }

    #[test]
    fn run_command__docker_shared__socket_at_its_path_with_host_and_group() {
        let args = args_for(&[], true);

        let expected = [
            (
                "--mount",
                "type=bind,src=/run/user/1000/docker.sock,dst=/run/user/1000/docker.sock",
            ),
            ("--env", "DOCKER_HOST=unix:///run/user/1000/docker.sock"),
            ("--env", "VZ_GROUPS=docker:969"),
        ];
        for (flag, value) in expected {
            assert!(has(&args, flag, value), "{flag} {value}: {args:?}");
        }
    }

    #[test]
    fn run_args__default__secure_floor() {
        let args = args_for(&[], false);

        let expected = [
            ("--cap-drop", "ALL"),
            ("--cap-add", "CHOWN"),
            ("--cap-add", "SETUID"),
            ("--cap-add", "SETGID"),
            ("--cap-add", "KILL"),
            ("--security-opt", "no-new-privileges"),
            ("--pids-limit", "512"),
        ];
        for (flag, value) in expected {
            assert!(has(&args, flag, value), "{flag} {value}: {args:?}");
        }
    }

    #[test]
    fn run_args__default__the_entrypoint_not_told_sudo() {
        let args = args_for(&[], false);

        assert!(!has_prefix(&args, "VZ_SUDO"), "{args:?}");
    }

    #[test]
    fn run_args__sudo__the_entrypoint_told() {
        let args = args_with(&[], false, true, false);

        assert!(has(&args, "--env", "VZ_SUDO=1"), "{args:?}");
    }

    #[test]
    fn run_args__sudo__docker_defaults_without_the_floor() {
        let args = args_with(&[], false, true, false);

        for flag in ["--cap-drop", "--cap-add", "--security-opt", "--pids-limit"] {
            assert!(!has_arg(&args, flag), "{flag}: {args:?}");
        }
    }

    #[test]
    fn run_args__shell_set__the_entrypoint_told() {
        let args = args_configured(&[], false, false, false, |session| {
            session.shell = Some("fish")
        });

        assert!(has(&args, "--env", "VZ_SHELL=fish"), "{args:?}");
    }

    #[test]
    fn run_args__shell_unset__the_entrypoint_not_told() {
        let args = args_for(&[], false);

        assert!(!has_prefix(&args, "VZ_SHELL"), "{args:?}");
    }

    /// The arguments of a session with two create hooks and one attach hook.
    fn hooked_args() -> Vec<String> {
        // The session borrows it for any lifetime the helper picks.
        let hooks = Box::leak(Box::new(EffectiveHooks {
            create: strings(&["npm ci", "echo \"a b\""]),
            attach: strings(&["git fetch"]),
        }));
        args_configured(&[], false, false, false, |session| session.hooks = hooks)
    }

    #[test]
    fn run_command__create_hooks__a_json_array_in_env() {
        let args = hooked_args();

        assert!(
            has(
                &args,
                "--env",
                r#"VZ_HOOKS_CREATE=["npm ci","echo \"a b\""]"#
            ),
            "{args:?}"
        );
    }

    #[test]
    fn run_command__attach_hooks__a_json_array_in_env() {
        let args = hooked_args();

        assert!(
            has(&args, "--env", r#"VZ_HOOKS_ATTACH=["git fetch"]"#),
            "{args:?}"
        );
    }

    #[test]
    fn run_command__no_hooks__no_hook_env() {
        let args = args_for(&[], false);

        assert!(!has_prefix(&args, "VZ_HOOKS"), "{args:?}");
    }

    #[test]
    fn run_args__host_network__the_host_s_network() {
        let args = args_with(&[], false, false, true);

        assert!(has(&args, "--network", "host"), "{args:?}");
    }

    /// On its own network the host is localhost: no alias.
    #[test]
    fn run_args__host_network__no_host_alias() {
        let args = args_with(&[], false, false, true);

        assert!(!has_arg(&args, "--add-host"), "{args:?}");
    }

    #[test]
    fn run_args__default__docker_s_network() {
        let args = args_with(&[], false, false, false);

        assert!(!has_arg(&args, "--network"), "{args:?}");
    }

    /// On docker's network the host has a name.
    #[test]
    fn run_args__default__the_host_alias() {
        let args = args_with(&[], false, false, false);

        assert!(
            has(&args, "--add-host", "host.docker.internal:host-gateway"),
            "{args:?}"
        );
    }

    #[test]
    fn create_args__any_session__named_labeled_with_a_fresh_session_dir() {
        let args = args_for(&[], true);

        let expected = [
            ("--name", "vz-0-vz"),
            ("--hostname", "vz-0-vz"),
            ("--label", "vz.index=0"),
            ("--tmpfs", "/run/viz-shell/session"),
        ];
        for (flag, value) in expected {
            assert!(has(&args, flag, value), "{flag} {value}: {args:?}");
        }
    }

    #[test]
    fn create_args__any_session__tells_the_shell_its_container() {
        let args = args_for(&[], false);

        assert!(has(&args, "--env", "VZ_CONTAINER=vz-0-vz"), "{args:?}");
    }

    #[test]
    fn create_args__a_configuration__tells_the_shell_its_name() {
        let args = args_configured(&[], false, false, false, |session| {
            session.config = Some("trusted")
        });

        assert!(
            has(&args, "--env", "VZ_CONTAINER_CONFIG=trusted"),
            "{args:?}"
        );
    }

    #[test]
    fn create_args__the_default__no_configuration_told() {
        let args = args_for(&[], false);

        assert!(!has_prefix(&args, "VZ_CONTAINER_CONFIG"), "{args:?}");
    }

    #[test]
    fn create_args__not_persistent__removed_on_exit_interactive_with_a_tty() {
        let args = args_for(&strings(&["id"]), true);

        for flag in ["--rm", "--interactive", "--tty"] {
            assert!(has_arg(&args, flag), "{flag}: {args:?}");
        }
    }

    #[test]
    fn create_args__persistent__neither_removed_nor_interactive() {
        let args = args_configured(&strings(&["id"]), true, false, false, |session| {
            session.persistent = true
        });

        for flag in ["--rm", "--interactive", "--tty"] {
            assert!(!has_arg(&args, flag), "{flag}: {args:?}");
        }
    }

    #[test]
    fn create_args__persistent__the_entrypoint_holds_for_shells_to_attach() {
        let args = args_configured(&strings(&["id"]), true, false, false, |session| {
            session.persistent = true
        });

        assert_eq!(
            args[args.len() - 3..],
            ["vz-vz:abc", "entrypoint", "--hold"]
        );
    }

    /// `vz enter` of `id -u` in `vz-0-vz`, on a terminal, with `TERM`
    /// passed through and `GH_TOKEN` configured.
    fn enter_args() -> Vec<String> {
        let passthrough = [("TERM".to_owned(), "xterm".to_owned())];
        let env_names = strings(&["GH_TOKEN"]);
        let command = strings(&["id", "-u"]);
        let enter = Enter {
            container: "vz-0-vz",
            workdir: Path::new("/home/sally/repos/vz"),
            tty: true,
            passthrough: &passthrough,
            env_names: &env_names,
            command: &command,
        };
        enter.args()
    }

    #[test]
    fn enter_args__configured_env__by_name_right_after_exec() {
        let args = enter_args();

        assert_eq!(&args[..3], ["exec", "--env", "GH_TOKEN"]);
    }

    #[test]
    fn enter_args__any__passthrough_and_workdir() {
        let args = enter_args();

        let expected = [
            ("--env", "TERM=xterm"),
            ("--workdir", "/home/sally/repos/vz"),
        ];
        for (flag, value) in expected {
            assert!(has(&args, flag, value), "{flag} {value}: {args:?}");
        }
    }

    #[test]
    fn enter_args__on_a_terminal__a_tty() {
        let args = enter_args();

        assert!(has_arg(&args, "--tty"), "{args:?}");
    }

    #[test]
    fn enter_args__command__vz_enters_the_container_as_the_user() {
        let args = enter_args();

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
        let command = strings(&["cargo", "test", "--"]);
        let enter = Enter {
            container: "vz-0-vz",
            workdir: Path::new("/"),
            tty: false,
            passthrough: &[],
            env_names: &[],
            command: &command,
        };
        let args = enter.args();

        let cli = Cli::try_parse_from(after(&args, ENTRYPOINT_PATH)).unwrap();

        let expected = Action::Enter {
            command: command.to_vec(),
        };
        assert_eq!(cli.action, Some(expected));
    }

    /// A vz container's mount table: `/`, `/proc`, `~/repos`, the
    /// repository, and vz's own binary.
    fn mount_table() -> Vec<PathBuf> {
        [
            "/",
            "/proc",
            "/home/sally/repos",
            "/home/sally/repos/vz",
            "/run/viz-shell/viz-shell",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }

    #[test]
    fn check_binary_reachable__outside_a_vz_container__accepted() {
        const OUTSIDE_A_VZ_CONTAINER: Option<&[PathBuf]> = None;

        let result = check_binary_reachable(
            Path::new("/run/viz-shell/viz-shell"),
            OUTSIDE_A_VZ_CONTAINER,
        );

        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn check_binary_reachable__on_a_same_path_mount__accepted() {
        let mounts = mount_table();
        let cases = [
            (
                "the repository",
                "/home/sally/repos/vz/target/x86_64-unknown-linux-musl/release/viz-shell",
            ),
            (
                "a sibling under ~/repos",
                "/home/sally/repos/other/viz-shell",
            ),
        ];
        for (case, binary) in cases {
            let result = check_binary_reachable(Path::new(binary), Some(&mounts));

            assert!(result.is_ok(), "{case}: {result:?}");
        }
    }

    #[test]
    fn check_binary_reachable__self_mount_or_the_image__refused() {
        let mounts = mount_table();
        let cases = [
            ("the self-mount", "/run/viz-shell/viz-shell"),
            ("the image itself", "/usr/local/bin/viz-shell"),
        ];
        for (case, binary) in cases {
            let result = check_binary_reachable(Path::new(binary), Some(&mounts));

            let message = format!("{:#}", result.unwrap_err());
            assert!(
                message
                    .contains("inside a vz container, run a viz-shell on a path the host has too"),
                "{case}: {message}"
            );
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

    /// Order is the rule here: a deeper mount lands on top of its parent.
    #[test]
    fn run_command__read_only_parent_of_the_repository__mounted_before_it() {
        let args = args_for(&[], true);

        let position = |mount: &str| args.iter().position(|arg| arg == mount).unwrap();
        let parent = position("type=bind,src=/home/sally/repos,dst=/home/sally/repos,readonly");
        let repository = position("type=bind,src=/home/sally/repos/vz,dst=/home/sally/repos/vz");
        assert!(parent < repository, "{args:?}");
    }
}
