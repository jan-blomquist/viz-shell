//! The `docker run` for a session: the repository at the same path as on the
//! host, the host's working directory, the state and host mounts, and the
//! host user, recreated by the entrypoint from the launcher's own binary.

use std::path::{Path, PathBuf};

use docker_wrapper::{DockerCommand, RunCommand};

use crate::constants::{CONTAINER_ROOT, ENTRYPOINT_PATH};
use crate::mounts::HostMount;
use crate::share::DockerSocket;
use crate::state::StateMount;
use crate::user::User;

pub struct Session<'a> {
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
    /// Empty for the shell.
    pub command: &'a [String],
    /// Whether stdin and stdout are a terminal.
    pub tty: bool,
}

impl Session<'_> {
    /// `docker run`'s arguments. docker-wrapper writes every `--env` as
    /// `NAME=VALUE`; the configured environment goes in as bare names, right
    /// after `run`, so values never reach a command line or a log.
    pub fn run_args(&self) -> Vec<String> {
        let mut args = self.run_command().build_command_args();
        let names = self
            .env_names
            .iter()
            .flat_map(|name| ["--env".to_owned(), name.clone()]);
        args.splice(1..1, names);
        args
    }

    fn run_command(&self) -> RunCommand {
        let entrypoint_args = ["entrypoint", "--"]
            .into_iter()
            .map(str::to_owned)
            .chain(self.command.iter().cloned())
            .collect();
        let mut run = RunCommand::new(self.image)
            .remove()
            .init()
            .interactive()
            .user(CONTAINER_ROOT)
            .entrypoint(ENTRYPOINT_PATH)
            .workdir(self.workdir)
            .cmd(entrypoint_args);
        run = self
            .binds()
            .iter()
            .fold(run, |run, bind| run.mount(bind.to_mount_arg()));
        if self.tty {
            run = run.tty();
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
                .map(|mount| Bind::new(&mount.path, &mount.path, mount.read_only)),
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
        let user = sally();
        let state = [StateMount {
            source: PathBuf::from(
                "/home/sally/repos/vz/.vz_state/home/sally/.config/opencode/opencode.json",
            ),
            target: PathBuf::from("/home/sally/.config/opencode/opencode.json"),
            kind: StateKind::File,
            init: None,
        }];
        let mounts = [HostMount {
            path: PathBuf::from("/home/sally/repos"),
            read_only: true,
        }];
        let docker = DockerSocket {
            path: PathBuf::from("/run/user/1000/docker.sock"),
            group: "docker".to_owned(),
            gid: 969,
        };
        let passthrough = [("TERM".to_owned(), "xterm-256color".to_owned())];
        let env_names = ["GH_TOKEN".to_owned()];
        let session = Session {
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
        };
        session.run_args()
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
        assert_eq!(&args[..3], ["run", "--env", "GH_TOKEN"]);
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
    fn run_command__read_only_parent_of_the_repository__mounted_before_it() {
        let args = args_for(&[], true);

        let position = |mount: &str| args.iter().position(|arg| arg == mount).unwrap();
        let parent = position("type=bind,src=/home/sally/repos,dst=/home/sally/repos,readonly");
        let repository = position("type=bind,src=/home/sally/repos/vz,dst=/home/sally/repos/vz");
        assert!(parent < repository, "{args:?}");
    }
}
