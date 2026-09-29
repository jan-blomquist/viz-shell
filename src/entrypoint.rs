//! `vz entrypoint`: the container's first process. It starts as root, adds
//! the host user to the image unless the image already has it, gives it its
//! home, then becomes that user and replaces itself with the command.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};
use tracing::debug;

use crate::constants::{GROUP_FILE, PASSWD_FILE, SHELLS};
use crate::user::{User, with_line};

/// Returns only on failure; on success the command replaces this process.
pub fn run(command: &[String]) -> anyhow::Result<()> {
    let user = User::from_env()?;
    let shell = default_shell()?;
    add_user(&user, &shell)?;
    prepare_home(&user)?;

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

/// The home may already exist, created as root by the engine to hold a
/// mount beneath it; only the home itself changes owner.
fn prepare_home(user: &User) -> anyhow::Result<()> {
    std::fs::create_dir_all(&user.home)
        .with_context(|| format!("creating {}", user.home.display()))?;
    std::os::unix::fs::chown(&user.home, Some(user.uid), Some(user.gid))
        .with_context(|| format!("giving {} to {}", user.home.display(), user.name))
}

fn user_command(user: &User, shell: &Path, command: &[String]) -> Command {
    let (program, args) = match command.split_first() {
        Some((program, args)) => (PathBuf::from(program), args),
        None => (shell.to_owned(), &[][..]),
    };
    let mut process = Command::new(program);
    process
        .args(args)
        .uid(user.uid)
        .gid(user.gid)
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
