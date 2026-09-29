//! `vz entrypoint`: the container's first process. It starts as root, adds
//! the host user to the image unless the image already has it, gives it its
//! home, then becomes that user and replaces itself with the command.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};
use nix::unistd::{Gid, Uid, setgid, setgroups, setuid};
use tracing::debug;

use crate::constants::{GROUP_FILE, GROUPS_ENV, MOUNTINFO_FILE, PASSWD_FILE, SHELLS};
use crate::user::{ExtraGroup, User, with_line};

/// Returns only on failure; on success the command replaces this process.
pub fn run(command: &[String]) -> anyhow::Result<()> {
    let user = User::from_env()?;
    let extra_groups = ExtraGroup::parse_list(&std::env::var(GROUPS_ENV).unwrap_or_default())?;
    let shell = default_shell()?;
    add_user(&user, &shell)?;
    add_extra_groups(&user, &extra_groups)?;
    prepare_home(&user)?;
    give_mount_parents(&user)?;

    become_user(&user, &extra_groups)?;
    let mut process = user_command(&user, &shell, command);
    debug!("exec {process:?} as {}", user.name);
    Err(anyhow!(process.exec())).context("starting the command")
}

fn add_user(user: &User, shell: &Path) -> anyhow::Result<()> {
    let passwd = read(PASSWD_FILE)?;
    if user.needs_user(&passwd)? {
        write(PASSWD_FILE, &with_line(&passwd, &user.passwd_line(shell)))?;
    }

    let group = read(GROUP_FILE)?;
    if user.needs_group(&group)? {
        write(GROUP_FILE, &with_line(&group, &user.group_line()))?;
    }
    Ok(())
}

/// Gives each extra group's gid a name in the image, so `id` shows it.
fn add_extra_groups(user: &User, groups: &[ExtraGroup]) -> anyhow::Result<()> {
    for group in groups {
        let group_file = read(GROUP_FILE)?;
        if let Some(line) = group.line_to_add(&group_file, &user.name) {
            write(GROUP_FILE, &with_line(&group_file, &line))?;
        }
    }
    Ok(())
}

/// Joins the user's own and extra groups, then becomes the user. Done here
/// rather than through `Command`: it drops every supplementary group when it
/// switches user, and cannot set them on stable Rust. Order matters: groups
/// and gid need root, so the uid goes last.
fn become_user(user: &User, extra_groups: &[ExtraGroup]) -> anyhow::Result<()> {
    let groups: Vec<Gid> = std::iter::once(user.gid)
        .chain(extra_groups.iter().map(|group| group.gid))
        .map(Gid::from_raw)
        .collect();
    setgroups(&groups).context("joining the user's groups")?;
    setgid(Gid::from_raw(user.gid)).context("switching to the user's group")?;
    setuid(Uid::from_raw(user.uid)).context("switching to the user")
}

/// The home may already exist: created as root by the engine to hold a mount
/// beneath it, or a host mount itself, which is left alone.
fn prepare_home(user: &User) -> anyhow::Result<()> {
    std::fs::create_dir_all(&user.home)
        .with_context(|| format!("creating {}", user.home.display()))?;
    give_if_root_owned(&user.home, user)
}

/// The engine creates the missing parents of each mount point as root. Under
/// the home they belong to the user: `~/repos` above the repository, `~/.local`
/// above a `~/.local/share/fish` state mount. Only root-owned ones change.
fn give_mount_parents(user: &User) -> anyhow::Result<()> {
    let mountinfo = read(MOUNTINFO_FILE)?;
    for dir in parents_below(&user.home, &mount_points(&mountinfo)) {
        give_if_root_owned(&dir, user)?;
    }
    Ok(())
}

/// Only a root-owned folder changes owner, so host mounts, read-only ones
/// included, are never touched.
fn give_if_root_owned(dir: &Path, user: &User) -> anyhow::Result<()> {
    let root_owned = std::fs::metadata(dir).is_ok_and(|meta| meta.uid() == 0);
    if root_owned {
        std::os::unix::fs::chown(dir, Some(user.uid), Some(user.gid))
            .with_context(|| format!("giving {} to {}", dir.display(), user.name))?;
    }
    Ok(())
}

/// The mount point of each line of `/proc/self/mountinfo`: the fifth field,
/// with the kernel's octal escapes (`\040` for a space) undone.
fn mount_points(mountinfo: &str) -> Vec<PathBuf> {
    mountinfo
        .lines()
        .filter_map(|line| line.split(' ').nth(4))
        .map(|field| PathBuf::from(OsString::from_vec(unescape_octal(field))))
        .collect()
}

fn unescape_octal(field: &str) -> Vec<u8> {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = bytes
            .get(i + 1..i + 4)
            .filter(|_| bytes[i] == b'\\')
            .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

/// Folders strictly between `home` and each mount point below it.
fn parents_below(home: &Path, mount_points: &[PathBuf]) -> BTreeSet<PathBuf> {
    mount_points
        .iter()
        .flat_map(|mount_point| {
            mount_point
                .ancestors()
                .skip(1)
                .take_while(|dir| *dir != home && dir.starts_with(home))
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn user_command(user: &User, shell: &Path, command: &[String]) -> Command {
    let (program, args) = match command.split_first() {
        Some((program, args)) => (PathBuf::from(program), args),
        None => (shell.to_owned(), &[][..]),
    };
    let mut process = Command::new(program);
    process
        .args(args)
        .env("USER", &user.name)
        .env("LOGNAME", &user.name)
        .env("SHELL", shell);
    process
}

fn default_shell() -> anyhow::Result<PathBuf> {
    SHELLS
        .iter()
        .map(PathBuf::from)
        .find(|shell| shell.exists())
        .with_context(|| format!("the image has none of {}", SHELLS.join(", ")))
}

fn read(path: &str) -> anyhow::Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {path}"))
}

fn write(path: &str, text: &str) -> anyhow::Result<()> {
    std::fs::write(path, text)
        .with_context(|| format!("writing {path}; vz entrypoint must start as root"))
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;

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

    #[test]
    fn user_command__no_command__runs_the_shell() {
        let process = user_command(&sally(), Path::new("/bin/bash"), &[]);

        assert_eq!(process.get_program(), "/bin/bash");
        assert_eq!(process.get_args().count(), 0);
    }

    #[test]
    fn user_command__command_given__runs_it_with_its_args() {
        let command = ["cargo".to_owned(), "test".to_owned()];

        let process = user_command(&sally(), Path::new("/bin/bash"), &command);

        assert_eq!(process.get_program(), "cargo");
        assert_eq!(process.get_args().collect::<Vec<_>>(), [OsStr::new("test")]);
    }

    #[test]
    fn user_command__any__names_the_user_and_shell() {
        let process = user_command(&sally(), Path::new("/bin/sh"), &[]);

        let env: BTreeMap<&OsStr, Option<&OsStr>> = process.get_envs().collect();
        let expected = BTreeMap::from([
            (OsStr::new("LOGNAME"), Some(OsStr::new("sally"))),
            (OsStr::new("SHELL"), Some(OsStr::new("/bin/sh"))),
            (OsStr::new("USER"), Some(OsStr::new("sally"))),
        ]);
        assert_eq!(env, expected);
    }

    #[test]
    fn mount_points__mountinfo_lines__fifth_field_unescaped() {
        let mountinfo = "\
            22 1 0:21 / / rw,relatime - overlay overlay rw\n\
            23 22 8:1 /src /home/sally/repos/app rw - ext4 /dev/sda1 rw\n\
            24 22 8:1 /x /home/sally/my\\040notes rw - ext4 /dev/sda1 rw\n";

        let points = mount_points(mountinfo);

        let expected: Vec<PathBuf> = ["/", "/home/sally/repos/app", "/home/sally/my notes"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(points, expected);
    }

    #[test]
    fn parents_below__mounts_inside_and_outside_home__only_folders_between() {
        let mount_points: Vec<PathBuf> = [
            "/",
            "/home/sally/repos/app",
            "/home/sally/.local/share/fish",
            "/home/sally/.config/opencode/opencode.json",
            "/var/cache/apt",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();

        let parents = parents_below(Path::new("/home/sally"), &mount_points);

        let expected: BTreeSet<PathBuf> = [
            "/home/sally/.config",
            "/home/sally/.config/opencode",
            "/home/sally/.local",
            "/home/sally/.local/share",
            "/home/sally/repos",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(parents, expected);
    }

    #[test]
    fn unescape_octal__escapes_and_plain_backslashes() {
        let cases: [(&str, &[u8]); 3] = [
            ("a\\040b", b"a b"),
            ("tab\\011", b"tab\t"),
            ("plain\\x", b"plain\\x"),
        ];
        for (field, expected) in cases {
            assert_eq!(unescape_octal(field), expected, "field: {field}");
        }
    }
}
