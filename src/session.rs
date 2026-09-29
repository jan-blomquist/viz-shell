//! The `docker run` for a session: the repository at the same path as on the
//! host, the host's working directory, the state and host mounts, and the
//! host user, recreated by the entrypoint from the launcher's own binary.

use std::path::Path;

use docker_wrapper::RunCommand;

use crate::constants::{CONTAINER_ROOT, ENTRYPOINT_PATH};
use crate::mounts::HostMount;
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
    /// Host variables to copy in, already filtered to those that are set.
    pub passthrough: &'a [(String, String)],
    /// Empty for the shell.
    pub command: &'a [String],
    /// Whether stdin and stdout are a terminal.
    pub tty: bool,
}

impl Session<'_> {
    pub fn run_command(&self) -> RunCommand {
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
        let user_env = self
            .user
            .env()
            .map(|(name, value)| (name.to_owned(), value));
        user_env
            .iter()
            .chain(self.passthrough)
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

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::path::PathBuf;

    use docker_wrapper::DockerCommand;

    use super::*;
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
            source: PathBuf::from("/home/sally/repos/vz/.vz_state/home/sally/.claude.json"),
            target: PathBuf::from("/home/sally/.claude.json"),
            kind: StateKind::File,
            init: None,
        }];
        let mounts = [HostMount {
            path: PathBuf::from("/home/sally/repos"),
            read_only: true,
        }];
        let passthrough = [("TERM".to_owned(), "xterm-256color".to_owned())];
        let session = Session {
            image: "vz-vz:abc",
            repo_root: Path::new("/home/sally/repos/vz"),
            workdir: Path::new("/home/sally/repos/vz/src"),
            vz_binary: Path::new("/home/sally/.cargo/bin/vz"),
            user: &user,
            state: &state,
            mounts: &mounts,
            passthrough: &passthrough,
            command,
            tty,
        };
        session.run_command().build_command_args()
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
                "type=bind,src=/home/sally/.cargo/bin/vz,dst=/run/vz/vz,readonly"
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
                "type=bind,src=/home/sally/repos/vz/.vz_state/home/sally/.claude.json,dst=/home/sally/.claude.json"
            ),
            "{args:?}"
        );
    }

    #[test]
    fn run_command__any_session__starts_the_entrypoint_as_root_in_the_workdir() {
        let args = args_for(&[], true);

        assert!(has(&args, "--user", "0:0"), "{args:?}");
        assert!(has(&args, "--entrypoint", "/run/vz/vz"), "{args:?}");
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

    #[test]
    fn run_command__no_terminal__omits_tty() {
        let args = args_for(&[], false);

        assert!(!args.contains(&"--tty".to_owned()), "{args:?}");
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
