//! The `docker run` for a session: the repository at the same path as on the
//! host, the host's working directory, and the host user, recreated by the
//! entrypoint from the launcher's own binary.

use std::path::Path;

use docker_wrapper::RunCommand;

use crate::constants::{CONTAINER_ROOT, ENTRYPOINT_PATH};
use crate::user::User;

pub struct Session<'a> {
    pub image: &'a str,
    pub repo_root: &'a Path,
    pub workdir: &'a Path,
    /// The launcher's own binary; static, so it runs in any image.
    pub vz_binary: &'a Path,
    pub user: &'a User,
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
            .mount(bind_mount(self.vz_binary, Path::new(ENTRYPOINT_PATH), true))
            .mount(bind_mount(self.repo_root, self.repo_root, false))
            .workdir(self.workdir)
            .cmd(entrypoint_args);
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

fn bind_mount(src: &Path, dst: &Path, read_only: bool) -> String {
    let read_only = if read_only { ",readonly" } else { "" };
    format!(
        "type=bind,src={},dst={}{read_only}",
        src.display(),
        dst.display()
    )
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::path::PathBuf;

    use docker_wrapper::DockerCommand;

    use super::*;

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
        let passthrough = [("TERM".to_owned(), "xterm-256color".to_owned())];
        let session = Session {
            image: "vz-vz:abc",
            repo_root: Path::new("/home/sally/repos/vz"),
            workdir: Path::new("/home/sally/repos/vz/src"),
            vz_binary: Path::new("/home/sally/.cargo/bin/vz"),
            user: &user,
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
    fn bind_mount__read_only_and_not() {
        let cases = [
            (false, "type=bind,src=/a,dst=/b"),
            (true, "type=bind,src=/a,dst=/b,readonly"),
        ];
        for (read_only, expected) in cases {
            assert_eq!(
                bind_mount(Path::new("/a"), Path::new("/b"), read_only),
                expected
            );
        }
    }
}
