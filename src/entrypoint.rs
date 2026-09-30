//! `vz entrypoint`: the container's first process. It starts as root, adds
//! the host user to the image unless the image already has it, gives it its
//! home, then enters: runs the hooks, becomes that user and replaces itself
//! with the command; or, in a persistent container, holds it open for shells
//! to attach.
//!
//! `vz enter`: an attached shell, through `docker exec`. It waits until the
//! entrypoint is ready, then enters the same way.
//!
//! `vz as-user`: a hook, run by either. It becomes the user, then replaces
//! itself with the hook's `sh -c`.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use nix::unistd::{Gid, Uid, setgid, setgroups, setuid};
use tracing::{debug, warn};

use crate::constants::{
    CREATED_DIR, ENTRYPOINT_PATH, FALLBACK_TERM, GROUP_FILE, GROUPS_ENV, HOOKS_ATTACH_ENV,
    HOOKS_CREATE_ENV, HOSTNAME_ADDRESS, HOSTNAME_FILE, HOSTS_FILE, MOUNTINFO_FILE, PASSWD_FILE,
    READY_FILE, REPO_ENV, SESSION_DIR, SHELL_ENV, SHELLS, SUDO_BINARIES, SUDO_ENV, SUDOERS_FILE,
    TERM_ENV, TERMINFO_DIRS,
};
use crate::user::{ExtraGroup, User, with_line};

/// How long `enter` waits for the entrypoint's setup.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_POLL: Duration = Duration::from_millis(20);

/// Returns only on failure; on success the command replaces this process, or,
/// with `hold`, this process holds the container open until it is stopped.
pub fn run(command: &[String], hold: bool) -> anyhow::Result<()> {
    let user = User::from_env()?;
    let extra_groups = extra_groups()?;
    let shell = choose_shell()?;
    add_user(&user, &shell)?;
    add_extra_groups(&user, &extra_groups)?;
    if let Err(error) = add_hostname() {
        warn!("the hostname may not resolve: {error:#}");
    }
    if std::env::var(SUDO_ENV).is_ok_and(|value| value == "1") {
        grant_sudo(&user)?;
    }
    prepare_home(&user)?;
    give_mount_parents(&user)?;
    clear_stale_claim(Path::new(CREATED_DIR))?;
    mark_ready()?;

    if hold {
        debug!("holding for shells to attach");
        // Stopping the container signals this process, which then ends.
        loop {
            std::thread::park();
        }
    }
    enter_as_user(&user, &extra_groups, &shell, command)
}

/// Returns only on failure; on success the command replaces this process.
pub fn enter(command: &[String]) -> anyhow::Result<()> {
    wait_ready()?;
    let user = User::from_env()?;
    let extra_groups = extra_groups()?;
    let shell = choose_shell()?;
    enter_as_user(&user, &extra_groups, &shell, command)
}

/// A hook's process: becomes the user, then replaces itself with the command.
pub fn as_user(command: &[String]) -> anyhow::Result<()> {
    let user = User::from_env()?;
    become_user(&user, &extra_groups()?)?;
    let (program, args) = command.split_first().context("no command")?;
    Err(anyhow!(Command::new(program).args(args).exec()))
        .with_context(|| format!("starting {program}"))
}

/// Every entry, the container's first included: the create hooks once per
/// container, the attach hooks, then the command as the user. Each has the
/// terminal, so hooks show their output and failures.
fn enter_as_user(
    user: &User,
    extra_groups: &[ExtraGroup],
    shell: &Path,
    command: &[String],
) -> anyhow::Result<()> {
    ensure_created(
        Path::new(CREATED_DIR),
        || run_hooks("create", HOOKS_CREATE_ENV, user, shell),
        |pid| Path::new(&format!("/proc/{pid}")).exists(),
    )?;
    run_hooks("attach", HOOKS_ATTACH_ENV, user, shell)?;
    exec_as_user(user, extra_groups, shell, command)
}

fn extra_groups() -> anyhow::Result<Vec<ExtraGroup>> {
    ExtraGroup::parse_list(&std::env::var(GROUPS_ENV).unwrap_or_default())
}

fn exec_as_user(
    user: &User,
    extra_groups: &[ExtraGroup],
    shell: &Path,
    command: &[String],
) -> anyhow::Result<()> {
    become_user(user, extra_groups)?;
    let mut process = user_command(user, shell, command);
    with_term(&mut process, &user.home);
    debug!("exec {process:?} as {}", user.name);
    Err(anyhow!(process.exec())).context("starting the command")
}

/// TERM one the image describes.
fn with_term(process: &mut Command, home: &Path) {
    let term = std::env::var(TERM_ENV).unwrap_or_default();
    if let Some(fallback) = term_fallback(&term, &terminfo_dirs(home), |path| path.exists()) {
        debug!("the image has no terminal description for {term}; TERM={fallback} instead");
        process.env(TERM_ENV, fallback);
    }
}

/// Two files mark the create hooks, in the container's writable layer: it
/// keeps them across a stop and start, and goes with the container. The
/// first entry claims `creating`, writing its pid, runs the hooks, then
/// renames it `created`; a failure removes it, so the next entry retries.
/// Another entry waits meanwhile, and takes over a claim whose process is
/// gone.
fn ensure_created(
    dir: &Path,
    create: impl FnOnce() -> anyhow::Result<()>,
    running: impl Fn(u32) -> bool,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let (creating, created) = (dir.join("creating"), dir.join("created"));
    let mut create = Some(create);
    loop {
        if created.exists() {
            return Ok(());
        }
        match claim(&creating) {
            Ok(()) if created.exists() => return remove(&creating),
            Ok(()) => {
                let create = create.take().expect("an entry claims at most once");
                return match create() {
                    Ok(()) => std::fs::rename(&creating, &created)
                        .with_context(|| format!("writing {}", created.display())),
                    Err(error) => {
                        remove(&creating)?;
                        Err(error)
                    }
                };
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                debug!("waiting for another entry's create hooks");
                wait_for_claim(&creating, &created, &running)?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("creating {}", creating.display()));
            }
        }
    }
}

fn claim(creating: &Path) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(creating)?;
    std::io::Write::write_all(&mut file, std::process::id().to_string().as_bytes())
}

/// Until the hooks are done, or the claim is gone: failed, or its process
/// ended without finishing. A claim without its pid yet is being written.
fn wait_for_claim(
    creating: &Path,
    created: &Path,
    running: impl Fn(u32) -> bool,
) -> anyhow::Result<()> {
    while !created.exists() {
        let holder = match std::fs::read_to_string(creating) {
            Ok(text) => text.trim().parse::<u32>().ok(),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", creating.display()));
            }
        };
        if holder.is_some_and(|pid| !running(pid)) {
            return remove(creating);
        }
        std::thread::sleep(READY_POLL);
    }
    Ok(())
}

/// A claim left by a start that stopped midway: at start, no entry runs.
fn clear_stale_claim(dir: &Path) -> anyhow::Result<()> {
    remove(&dir.join("creating"))
}

/// Gone already is fine.
fn remove(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Each command of `env_var`'s list, in order, through `sh -c` as the user,
/// in the repository; the first that fails stops the entry.
fn run_hooks(kind: &str, env_var: &str, user: &User, shell: &Path) -> anyhow::Result<()> {
    let commands = parse_hooks(std::env::var_os(env_var).as_deref())
        .with_context(|| format!("reading {env_var}"))?;
    if commands.is_empty() {
        return Ok(());
    }
    let repo = std::env::var_os(REPO_ENV)
        .with_context(|| format!("{REPO_ENV} is unset; hooks run in the repository"))?;
    let sh = find_program("sh", &std::env::var("PATH").unwrap_or_default())
        .context("hooks run through sh, which the image lacks")?;
    for command in &commands {
        debug!("hook ({kind}): {command}");
        let mut process = hook_command(user, shell, &sh, Path::new(&repo), command);
        with_term(&mut process, &user.home);
        let status = process
            .status()
            .with_context(|| format!("starting {kind} hook `{command}`"))?;
        if !status.success() {
            let how = match status.code() {
                Some(code) => format!("exit status {code}"),
                None => format!("signal {}", status.signal().unwrap_or_default()),
            };
            bail!("{kind} hook `{command}`: {how}");
        }
    }
    Ok(())
}

/// A hook list as the launcher passes it: a JSON array; none when unset.
fn parse_hooks(value: Option<&OsStr>) -> anyhow::Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let text = value.to_str().context("not UTF-8")?;
    serde_json::from_str(text).context("not a JSON array of commands")
}

/// `vz as-user -- sh -c COMMAND` in the repository: a child can't join the
/// user's extra groups through `Command` on stable Rust, so it becomes the
/// user itself, as the entrypoint does.
fn hook_command(user: &User, shell: &Path, sh: &Path, repo: &Path, command: &str) -> Command {
    let command = [ENTRYPOINT_PATH, "as-user", "--"]
        .into_iter()
        .map(str::to_owned)
        .chain([
            sh.display().to_string(),
            "-c".to_owned(),
            command.to_owned(),
        ])
        .collect::<Vec<_>>();
    let mut process = user_command(user, shell, &command);
    process.current_dir(repo);
    process
}

/// The session folder is a tmpfs, empty on every start: the mark is this
/// start's.
fn mark_ready() -> anyhow::Result<()> {
    std::fs::create_dir_all(SESSION_DIR).with_context(|| format!("creating {SESSION_DIR}"))?;
    std::fs::write(READY_FILE, "").with_context(|| format!("writing {READY_FILE}"))
}

fn wait_ready() -> anyhow::Result<()> {
    let started = Instant::now();
    while !Path::new(READY_FILE).exists() {
        if started.elapsed() > READY_TIMEOUT {
            bail!(
                "the container's entrypoint has not set it up after {}s; see `docker logs` for it",
                READY_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(READY_POLL);
    }
    Ok(())
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

/// Makes the hostname resolve. With `share.host_network` it is the host's,
/// and the hosts file Docker copies from the host may not list it (systemd
/// resolves it there); sudo would warn on every use.
fn add_hostname() -> anyhow::Result<()> {
    let hostname = read(HOSTNAME_FILE)?;
    let hosts = read(HOSTS_FILE)?;
    if let Some(line) = hostname_line(&hosts, hostname.trim()) {
        write(HOSTS_FILE, &with_line(&hosts, &line))?;
    }
    Ok(())
}

/// The hosts line for `hostname`, unless the hosts file already names it.
fn hostname_line(hosts: &str, hostname: &str) -> Option<String> {
    let listed = hosts
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .any(|line| line.split_whitespace().skip(1).any(|name| name == hostname));
    (!hostname.is_empty() && !listed).then(|| format!("{HOSTNAME_ADDRESS}\t{hostname}"))
}

/// `privileges.sudo`: a sudoers line for the user, when the image has sudo.
/// Without it, a warning: the capabilities are there, but no way to use them.
fn grant_sudo(user: &User) -> anyhow::Result<()> {
    if !SUDO_BINARIES.iter().any(|path| Path::new(path).exists()) {
        warn!(
            "privileges.sudo is granted, but the image has no sudo; install it in the \
             Dockerfile to use it"
        );
        return Ok(());
    }
    let file = Path::new(SUDOERS_FILE);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(file, sudoers_line(&user.name))
        .with_context(|| format!("writing {}", file.display()))?;
    // sudo ignores a sudoers file anyone but root can write.
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o440))
        .with_context(|| format!("setting the mode of {}", file.display()))
}

fn sudoers_line(user: &str) -> String {
    format!("{user} ALL=(ALL) NOPASSWD:ALL\n")
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
pub fn mount_points(mountinfo: &str) -> Vec<PathBuf> {
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

/// Where to look for terminal descriptions, as ncurses does: `$TERMINFO`,
/// `~/.terminfo`, `$TERMINFO_DIRS` (an empty entry for the defaults), then
/// the defaults.
fn terminfo_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("TERMINFO")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    dirs.push(home.join(".terminfo"));
    if let Some(listed) = std::env::var_os("TERMINFO_DIRS") {
        dirs.extend(std::env::split_paths(&listed).filter(|dir| !dir.as_os_str().is_empty()));
    }
    dirs.extend(TERMINFO_DIRS.iter().map(PathBuf::from));
    dirs
}

/// `FALLBACK_TERM` when `term` names a terminal none of `dirs` describes;
/// `None` when it is described, or unset. A description sits under the
/// name's first letter, or its hex code: `x/xterm-ghostty`, `78/xterm-ghostty`.
fn term_fallback(
    term: &str,
    dirs: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Option<&'static str> {
    let first = term.chars().next()?;
    let letters = [first.to_string(), format!("{:x}", u32::from(first))];
    let described = dirs
        .iter()
        .flat_map(|dir| {
            letters
                .iter()
                .map(move |letter| dir.join(letter).join(term))
        })
        .any(|entry| exists(&entry));
    (!described && term != FALLBACK_TERM).then_some(FALLBACK_TERM)
}

/// The configured `shell` if the image has it; else, with a warning when one
/// was configured, the first of bash and sh.
fn choose_shell() -> anyhow::Result<PathBuf> {
    let wanted = std::env::var(SHELL_ENV)
        .ok()
        .filter(|shell| !shell.is_empty());
    let search_path = std::env::var("PATH").unwrap_or_default();
    if let Some(shell) = wanted
        .as_deref()
        .and_then(|name| find_program(name, &search_path))
    {
        return Ok(shell);
    }
    let fallback = SHELLS
        .iter()
        .find_map(|shell| find_program(shell, &search_path))
        .with_context(|| format!("the image has none of {}", SHELLS.join(", ")))?;
    if let Some(wanted) = wanted {
        warn!(
            "shell `{wanted}` is not in the image, so {} instead; install it in the \
             Dockerfile",
            fallback.display()
        );
    }
    Ok(fallback)
}

/// An executable: an absolute path as it is, a name in the first
/// `search_path` folder that has it; `None` if missing.
fn find_program(name: &str, search_path: &str) -> Option<PathBuf> {
    which::which_in(name, Some(search_path), "/").ok()
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

    use clap::Parser;

    use super::*;
    use crate::cli::{Action, Cli};

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
    fn hostname_line__hosts_file__only_when_the_hostname_is_missing() {
        let hosts = "127.0.0.1\tlocalhost\n# 127.0.1.1 box\n172.17.0.2\tabc123 other\n";
        let cases = [
            ("box", Some("127.0.1.1\tbox".to_owned())),
            ("abc123", None),
            ("other", None),
            ("localhost", None),
            ("", None),
        ];

        for (hostname, expected) in cases {
            assert_eq!(hostname_line(hosts, hostname), expected, "{hostname}");
        }
    }

    #[test]
    fn sudoers_line__user__without_a_password() {
        assert_eq!(sudoers_line("sally"), "sally ALL=(ALL) NOPASSWD:ALL\n");
    }

    #[test]
    fn find_program__name_or_path__an_executable_where_it_is() {
        let root = tempfile::tempdir().unwrap();
        let dir = |name: &str| {
            let dir = root.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };
        let (local, usr, bin) = (dir("local"), dir("usr"), dir("bin"));
        let file = |dir: &Path, name: &str, mode: u32| {
            let path = dir.join(name);
            std::fs::write(&path, "").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            path
        };
        let local_fish = file(&local, "fish", 0o755);
        let usr_fish = file(&usr, "fish", 0o755);
        let zsh = file(&bin, "zsh", 0o755);
        let not_executable = file(&bin, "nu", 0o644);
        let search_path = format!("{}:{}::{}", local.display(), usr.display(), bin.display());
        let cases = [
            ("fish", Some(&local_fish)),
            ("zsh", Some(&zsh)),
            (usr_fish.to_str().unwrap(), Some(&usr_fish)),
            ("nu", None),
            (not_executable.to_str().unwrap(), None),
            ("bash", None),
        ];

        for (name, expected) in cases {
            assert_eq!(
                find_program(name, &search_path).as_ref(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn term_fallback__described_or_not() {
        let dirs = [
            PathBuf::from("/usr/share/terminfo"),
            PathBuf::from("/lib/terminfo"),
        ];
        let present = [
            "/lib/terminfo/x/xterm-256color",
            "/usr/share/terminfo/78/xterm-kitty",
            "/lib/terminfo/d/dumb",
        ];
        let exists = |path: &Path| present.iter().any(|entry| Path::new(entry) == path);
        let cases = [
            ("xterm-256color", None),
            ("xterm-kitty", None),
            ("dumb", None),
            ("xterm-ghostty", Some("xterm-256color")),
            ("", None),
        ];
        for (term, expected) in cases {
            assert_eq!(term_fallback(term, &dirs, exists), expected, "{term:?}");
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
    fn parse_hooks__env_value__commands_in_order_or_refused() {
        let cases: [(Option<&str>, Option<&[&str]>); 5] = [
            (None, Some(&[])),
            (
                Some(r#"["npm ci","echo \"a b\""]"#),
                Some(&["npm ci", "echo \"a b\""]),
            ),
            (Some("[]"), Some(&[])),
            (Some("npm ci"), None),
            (Some(r#"{"create":"npm ci"}"#), None),
        ];
        for (value, expected) in cases {
            let parsed = parse_hooks(value.map(OsStr::new)).ok();

            let expected =
                expected.map(|commands| commands.iter().map(|c| c.to_string()).collect());
            assert_eq!(parsed, expected, "{value:?}");
        }
    }

    fn npm_ci_hook() -> Command {
        hook_command(
            &sally(),
            Path::new("/bin/bash"),
            Path::new("/bin/sh"),
            Path::new("/home/sally/repos/app"),
            "npm ci",
        )
    }

    #[test]
    fn hook_command__command__sh_c_as_the_user_in_the_repository() {
        let process = npm_ci_hook();

        assert_eq!(process.get_program(), ENTRYPOINT_PATH);
        assert_eq!(
            process.get_args().collect::<Vec<_>>(),
            ["as-user", "--", "/bin/sh", "-c", "npm ci"].map(OsStr::new)
        );
        assert_eq!(
            process.get_current_dir(),
            Some(Path::new("/home/sally/repos/app"))
        );
        let env: BTreeMap<&OsStr, Option<&OsStr>> = process.get_envs().collect();
        assert_eq!(env[OsStr::new("USER")], Some(OsStr::new("sally")));
        assert_eq!(env[OsStr::new("SHELL")], Some(OsStr::new("/bin/bash")));
    }

    /// The hook's arguments after the binary parse back into `as-user`.
    #[test]
    fn hook_command__args__parse_as_as_user() {
        let process = npm_ci_hook();
        let args = process
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned());

        let cli = Cli::try_parse_from(std::iter::once("vz".to_owned()).chain(args)).unwrap();

        let command = ["/bin/sh", "-c", "npm ci"].map(str::to_owned).to_vec();
        assert_eq!(cli.action, Some(Action::AsUser { command }));
    }

    /// What `ensure_created` did: whether it succeeded, how often the hooks
    /// ran, and the marker files it left.
    #[derive(Debug, PartialEq)]
    struct Created {
        ok: bool,
        runs: usize,
        left: Vec<String>,
    }

    /// `ensure_created` in a temporary folder that already holds `files`,
    /// with hooks that succeed or not.
    fn ensure_created_in(files: &[(&str, &str)], hooks_succeed: bool) -> Created {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        let mut runs = 0;
        let create = || {
            runs += 1;
            match hooks_succeed {
                true => Ok(()),
                false => Err(anyhow!("create hook `exit 3`: exit status 3")),
            }
        };
        // No process is running but this one.
        let result = ensure_created(dir.path(), create, |pid| pid == std::process::id());
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        Created {
            ok: result.is_ok(),
            runs,
            left,
        }
    }

    #[test]
    fn ensure_created__markers__hooks_run_once_per_container() {
        let created = |ok: bool, runs: usize, left: &[&str]| Created {
            ok,
            runs,
            left: left.iter().map(|file| file.to_string()).collect(),
        };
        let cases = [
            (
                "the first entry",
                &[][..],
                true,
                created(true, 1, &["created"]),
            ),
            (
                "created already",
                &[("created", "")],
                true,
                created(true, 0, &["created"]),
            ),
            (
                "a failure, retried next",
                &[],
                false,
                created(false, 1, &[]),
            ),
            // Its process ended without finishing: taken over.
            (
                "a stale claim",
                &[("creating", "4194305")],
                true,
                created(true, 1, &["created"]),
            ),
            (
                "created wins over a claim",
                &[("creating", "4194305"), ("created", "")],
                true,
                created(true, 0, &["created", "creating"]),
            ),
        ];
        for (name, files, hooks_succeed, expected) in cases {
            let actual = ensure_created_in(files, hooks_succeed);

            assert_eq!(actual, expected, "{name}");
        }
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
