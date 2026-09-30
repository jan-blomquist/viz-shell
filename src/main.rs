mod banner;
mod build;
mod cli;
mod config;
mod constants;
mod containers;
mod engine;
mod entrypoint;
mod env;
mod mounts;
mod repo;
mod session;
mod share;
mod state;
mod user;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use clap::Parser;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

use crate::build::ImageStep;
use crate::cli::{Action, Cli};
use crate::config::{Config, ConfigFile, Layer};
use crate::constants::{
    CONTAINER_ENV, CONTAINER_PROFILE_ENV, DEFAULT_LOG_FILTER, DEFAULT_STATE_DIR, DOCKER_HOST_ENV,
    ENTRYPOINT_PATH, GID_ENV, GLOBAL_CONFIG_DIR, GLOBAL_CONFIG_FILE, GROUP_ENV, GROUPS_ENV,
    HOME_ENV, HOOKS_ATTACH_ENV, HOOKS_CREATE_ENV, LOG_ENV, MOUNTINFO_FILE, PASSTHROUGH_ENV,
    REPO_CONFIG_FILES, REPO_ENV, SHELL_ENV, SUDO_ENV, UID_ENV, USER_ENV,
};
use crate::containers::{Container, Target};
use crate::engine::{Created, Engine};
use crate::env::CliEnv;
use crate::session::{Enter, Session};
use crate::share::DockerSocket;
use crate::user::User;

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match &cli.action {
        // Before any runtime starts: the entrypoint changes user and execs,
        // which wants a single-threaded process.
        Some(Action::Entrypoint { hold, command }) => entrypoint::run(command, *hold),
        Some(Action::Enter { command }) => entrypoint::enter(command),
        Some(Action::AsUser { command }) => entrypoint::as_user(command),
        Some(Action::Profiles) => print_profiles(&load_with(cli.config_file.as_deref())?),
        Some(Action::Ls { all }) => runtime()?.block_on(list(*all)),
        Some(Action::Kill { targets, all }) => runtime()?.block_on(kill(targets, *all)),
        Some(Action::New { name, command }) => launched(&cli, Launch::New(name), command),
        Some(Action::Attach { target, command }) => {
            let target = target.as_deref().map(Target::parse);
            launched(&cli, Launch::Attach(target), command)
        }
        None => launched(&cli, Launch::Default, &cli.command),
    }
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Runtime::new().context("starting the async runtime")
}

/// Exits with the shell's, or the command's, exit code.
fn launched(cli: &Cli, how: Launch, command: &[String]) -> anyhow::Result<()> {
    let exit_code = runtime()?.block_on(launch(cli, how, command))?;
    std::process::exit(exit_code);
}

/// The repository, the host user, and the configuration files in effect.
struct Loaded {
    repo_root: PathBuf,
    user: User,
    global_file: Option<PathBuf>,
    repo_file: Option<PathBuf>,
    local_file: Option<PathBuf>,
    config: Config,
}

impl Loaded {
    /// The files read, each with its origin: global, repo, local, in that order.
    fn files(&self) -> Vec<(ConfigFile, &Path)> {
        [
            (ConfigFile::Global, &self.global_file),
            (ConfigFile::Repository, &self.repo_file),
            (ConfigFile::Local, &self.local_file),
        ]
        .into_iter()
        .filter_map(|(file, path)| Some((file, path.as_deref()?)))
        .collect()
    }

    /// `# global …, repo …, local …`: the files read.
    fn files_line(&self) -> String {
        let files: Vec<String> = self
            .files()
            .into_iter()
            .map(|(file, path)| format!("{} {}", file.origin(), path.display()))
            .collect();
        files.join(", ")
    }
}

/// Reads the global configuration and the repository's, either optional
/// but not both, then your local overlay of the repository's, if any. `-c`
/// names the repository's; its overlay is next to it.
fn load_with(config_file: Option<&Path>) -> anyhow::Result<Loaded> {
    let repo_root = repo::root()?;
    let user = User::of_host()?;
    debug!("host user: {user:?}");
    let global_path = global_config_file()?;
    match config::scaffold_global(&global_path) {
        Ok(true) => info!(
            "wrote the default global configuration to {}: edit it to taste",
            global_path.display()
        ),
        Ok(false) => {}
        Err(error) => warn!("no global configuration: {error:#}"),
    }
    let global_file = Some(global_path.clone()).filter(|file| file.is_file());
    let repo_file = match config_file {
        Some(file) => Some(
            std::path::absolute(file).with_context(|| format!("resolving {}", file.display()))?,
        ),
        None => repo::config_file(&repo_root),
    };
    ensure!(
        global_file.is_some() || repo_file.is_some(),
        "no configuration: add one of {} at the git root, or {}",
        REPO_CONFIG_FILES.join(", "),
        global_path.display()
    );
    let local_file = match config_file {
        Some(_) => repo_file
            .as_deref()
            .map(repo::local_file_for)
            .filter(|file| file.is_file()),
        None => repo::local_config_file(&repo_root),
    };
    if let Some(file) = &local_file
        && repo::is_tracked(file)
    {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        warn!("{name} is tracked by git: it is meant to be personal");
    }
    let read = |file: &Option<PathBuf>| {
        file.as_deref()
            .map(|file| Layer::load(file, &user.home, &repo_root))
            .transpose()
    };
    let config = Config::new(read(&global_file)?, read(&repo_file)?, read(&local_file)?)?;
    Ok(Loaded {
        repo_root,
        user,
        global_file,
        repo_file,
        local_file,
        config,
    })
}

/// `$XDG_CONFIG_HOME/viz-shell/global.yml`, or under `~/.config` without it.
fn global_config_file() -> anyhow::Result<PathBuf> {
    use etcetera::BaseStrategy;
    let strategy = etcetera::choose_base_strategy().context("finding your home folder")?;
    Ok(strategy
        .config_dir()
        .join(GLOBAL_CONFIG_DIR)
        .join(GLOBAL_CONFIG_FILE))
}

/// Each profile: the files that define it, what it extends and changes;
/// then the configuration files read.
fn print_profiles(loaded: &Loaded) -> anyhow::Result<()> {
    let profiles = loaded.config.profiles();
    if profiles.is_empty() {
        println!("No profiles yet: add them under `profiles:` in any file.");
    } else {
        let rows = profiles.into_iter().map(|profile| {
            [
                profile.name,
                profile.defined_in.join(", "),
                profile.extends.unwrap_or_else(|| "-".to_owned()),
                Some(profile.changes.join(", "))
                    .filter(|changes| !changes.is_empty())
                    .unwrap_or_else(|| "-".to_owned()),
            ]
        });
        print_table(["PROFILE", "FROM", "EXTENDS", "CHANGES"], rows);
    }
    println!();
    let rows = loaded.files().into_iter().map(|(file, path)| {
        [
            file.origin().to_owned(),
            config::tilde(path, &loaded.user.home),
        ]
    });
    print_table(["FROM", "FILE"], rows);
    Ok(())
}

/// Rows under a header, in columns as wide as their widest cell, like `docker ps`.
fn print_table<const N: usize>(header: [&str; N], rows: impl IntoIterator<Item = [String; N]>) {
    let rows: Vec<[String; N]> = std::iter::once(header.map(str::to_owned))
        .chain(rows)
        .collect();
    let widths: Vec<usize> = (0..N)
        .map(|column| rows.iter().map(|row| row[column].len()).max().unwrap_or(0))
        .collect();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:width$}"))
            .collect();
        println!("{}", cells.join("   ").trim_end());
    }
}

/// What a launch does: create a container, or attach to one.
enum Launch<'a> {
    /// A plain `vz`: joins a container with `attach: true` when one fits,
    /// else creates one.
    Default,
    /// `vz new NAME`.
    New(&'a str),
    /// `vz attach [TARGET]`.
    Attach(Option<Target>),
}

/// Runs the shell, or `command`, as the host user in a container of the
/// repository: a new one, or one that runs; returns the exit code.
async fn launch(cli: &Cli, how: Launch<'_>, command: &[String]) -> anyhow::Result<i32> {
    ensure!(
        cfg!(target_env = "musl"),
        "vz mounts itself into the container, so it must be a static musl build: \
         cargo build --target x86_64-unknown-linux-musl"
    );
    let loaded = load_with(cli.config_file.as_deref())?;
    let profile = cli.profile.as_deref();
    let config = loaded
        .config
        .effective(profile)
        .context("in the configuration")?;
    let header = format!(
        "{}; layers: {}",
        loaded.files_line(),
        config.layers.join(", ")
    );
    if cli.show_effective_config {
        println!("# effective configuration of {header}");
        for line in build::describe(&config.images, &loaded.repo_root, &loaded.user.home) {
            println!("# image: {line}");
        }
        print!("{}", config.to_yaml()?);
        return Ok(0);
    }
    debug!("effective configuration: {config:?}");
    let config_files: Vec<(ConfigFile, PathBuf)> = loaded
        .files()
        .into_iter()
        .map(|(file, path)| (file, path.to_owned()))
        .collect();
    let Loaded {
        repo_root, user, ..
    } = loaded;
    // Paths are absolute by now; the repository root is the base of the rest.
    let config_dir = repo_root.clone();

    let cli_env = cli
        .env
        .iter()
        .map(|arg| CliEnv::parse(arg))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let env_paths = env::Paths {
        config_dir: &config_dir,
        repo_root: &repo_root,
        home: &user.home,
    };
    let environment = env::without_reserved(
        env::plan(&config.env, &cli_env, &env_paths)?,
        &reserved_env_names(),
    );
    if cli.show_env {
        print_env(&header, &environment);
        return Ok(0);
    }

    let state_dir = match &config.state_dir {
        Some(dir) => config::resolve_host_path(dir, &config_dir, &user.home),
        None => repo_root.join(DEFAULT_STATE_DIR),
    };
    let state = state::plan(&config.state, &user.home, &state_dir, &repo_root)?;
    let mounts = mounts::plan(&config.mounts, &user.home, &state, &repo_root)?;
    mounts::check_sources_exist(&mounts)?;

    let vz_binary = std::env::current_exe().context("locating the vz binary")?;
    let mount_points = match Path::new(ENTRYPOINT_PATH).exists() {
        true => Some(entrypoint::mount_points(
            &std::fs::read_to_string(MOUNTINFO_FILE).context("reading the mount table")?,
        )),
        false => None,
    };
    session::check_binary_reachable(&vz_binary, mount_points.as_deref())?;

    // Before anything is created on the host: a failed start leaves nothing.
    let engine = Engine::detect().await?;
    let docker = match config.share.docker {
        true => Some(DockerSocket::locate(&engine.docker_endpoint().await?)?),
        false => None,
    };
    let existing = engine.containers(Some(&repo_root)).await?;
    let config_hash = containers::config_hash(&config.to_yaml()?);
    let joining = match &how {
        Launch::Attach(target) => {
            let container = match target {
                Some(target) => containers::find(&existing, target)?,
                None => containers::only_running(&existing)?,
            };
            containers::check_profile(container, profile)?;
            Some(container)
        }
        Launch::Default if config.attach => containers::to_join(&existing, profile),
        Launch::Default => None,
        Launch::New(name) => {
            containers::check_session_name(name)?;
            if let Ok(taken) = containers::find(&existing, &Target::Name(name.to_string())) {
                bail!("{} exists; attach with `vz attach {name}`", taken.name);
            }
            None
        }
    };

    let workdir = std::env::current_dir().context("reading the current directory")?;
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let passthrough = passthrough_env();
    let env_names: Vec<String> = environment.keys().cloned().collect();
    let enter = |container| Enter {
        container,
        workdir: &workdir,
        tty,
        passthrough: &passthrough,
        env_names: &env_names,
        command,
    };
    let show_banner =
        |container: &str, image: &str, image_bases: &[&str], attached: bool, persistent: bool| {
            if !(config.banner && command.is_empty() && tty) {
                return;
            }
            let branch = repo::branch(&repo_root);
            let config_files: Vec<(ConfigFile, &Path)> = config_files
                .iter()
                .map(|(file, path)| (*file, path.as_path()))
                .collect();
            let facts = banner::Session {
                user: &user.name,
                container,
                attached,
                persistent,
                home: &user.home,
                repo_root: &repo_root,
                branch: branch.as_deref(),
                config_files: &config_files,
                profile,
                image,
                image_bases,
                shell: config.shell.as_deref(),
                sudo: config.privileges.sudo,
                docker: docker.as_ref().map(|socket| socket.path.as_path()),
                host_network: config.share.host_network,
                mounts: &mounts,
                state_paths: state.len(),
                state_dir: &state_dir,
                env_vars: environment.len(),
                create_hooks: config.hooks.create.len(),
                attach_hooks: config.hooks.attach.len(),
            };
            anstream::print!(
                "{}",
                banner::render(&banner::title(&facts), &banner::facts(&facts))
            );
        };
    let values: Vec<(String, String)> = environment
        .iter()
        .map(|(name, var)| (name.clone(), var.value.clone()))
        .collect();

    if let Some(container) = joining {
        if container.config != config_hash {
            warn!(
                "the configuration changed since {} was created; `vz kill {}` and start \
                 again to apply it",
                container.name, container.index
            );
        }
        if !container.running {
            info!("starting {}", container.name);
            engine.start(&container.name).await?;
        }
        show_banner(
            &container.name,
            &container.image,
            &[],
            true,
            container.persistent,
        );
        return engine.exec(enter(&container.name).args(), &values).await;
    }

    state::create_sources(&state)?;
    mounts::create_points_in_state(&mounts)?;
    let steps = build::plan(&config.images, &config_dir, &user)?;
    prepare_image(&engine, &steps).await?;
    let (top, below) = steps.split_last().expect("an image chain has a top");
    let image = top.tag().to_owned();
    let image_bases: Vec<&str> = below.iter().rev().map(ImageStep::tag).collect();
    let session_name = match how {
        Launch::New(name) => Some(name),
        _ => None,
    };
    let repo_dir = repo::dir_name(&repo_root);
    let mut taken = Vec::new();
    let name = loop {
        let index = containers::next_index(&existing, &taken);
        let name = containers::container_name(index, &repo_dir, session_name);
        let labels = containers::labels(
            &repo_root,
            index,
            session_name,
            profile,
            config.persistent,
            &config_hash,
        );
        let session = Session {
            name: &name,
            profile,
            labels: &labels,
            persistent: config.persistent,
            image: &image,
            repo_root: &repo_root,
            workdir: &workdir,
            vz_binary: &vz_binary,
            user: &user,
            state: &state,
            mounts: &mounts,
            docker: docker.as_ref(),
            passthrough: &passthrough,
            env_names: &env_names,
            command,
            tty,
            sudo: config.privileges.sudo,
            host_network: config.share.host_network,
            shell: config.shell.as_deref(),
            hooks: &config.hooks,
        };
        match engine.create(session.create_args(), &values).await? {
            Created::Yes => break name,
            // Another vz took the index meanwhile: the next one.
            Created::NameTaken if taken.len() < MAX_NAME_RETRIES => taken.push(index),
            Created::NameTaken => bail!("{name} is taken, and so are the next names; `vz ls`"),
        }
    };
    if config.persistent {
        engine.start(&name).await?;
        show_banner(&name, &image, &image_bases, false, true);
        engine.exec(enter(&name).args(), &values).await
    } else {
        show_banner(&name, &image, &image_bases, false, false);
        engine.start_attached(&name).await
    }
}

/// How often a launch tries the next index when another vz took one.
const MAX_NAME_RETRIES: usize = 10;

/// `vz ls`: this repository's containers, or every repository's.
async fn list(all: bool) -> anyhow::Result<()> {
    let repo_root = match all {
        true => None,
        false => Some(repo::root()?),
    };
    let engine = Engine::detect().await?;
    let found = engine.containers(repo_root.as_deref()).await?;
    if found.is_empty() {
        let whose = if all { "vz" } else { "this repository" };
        println!("No containers of {whose}.");
        return Ok(());
    }
    let row = |container: &Container| {
        [
            container.name.clone(),
            container.profile.clone().unwrap_or_else(|| "-".to_owned()),
            (if container.persistent { "yes" } else { "no" }).to_owned(),
            (if container.running {
                "running"
            } else {
                "stopped"
            })
            .to_owned(),
            container.created.clone(),
        ]
    };
    let header = ["NAME", "PROFILE", "PERSISTENT", "STATE", "CREATED"];
    if all {
        let home = User::of_host()?.home;
        let rows = found.iter().map(|container| {
            let [name, profile, persistent, state, created] = row(container);
            let repo = config::tilde(&container.repo, &home);
            [name, profile, persistent, state, created, repo]
        });
        let [name, profile, persistent, state, created] = header;
        print_table([name, profile, persistent, state, created, "REPO"], rows);
    } else {
        print_table(header, found.iter().map(row));
    }
    Ok(())
}

/// `vz kill`: removes the named containers of this repository, or all of them.
async fn kill(targets: &[String], all: bool) -> anyhow::Result<()> {
    let repo_root = repo::root()?;
    let engine = Engine::detect().await?;
    let existing = engine.containers(Some(&repo_root)).await?;
    let chosen: Vec<&Container> = match all {
        true => existing.iter().collect(),
        false => targets
            .iter()
            .map(|target| containers::find(&existing, &Target::parse(target)))
            .collect::<anyhow::Result<_>>()?,
    };
    if chosen.is_empty() {
        println!("No containers of this repository.");
        return Ok(());
    }
    let names: Vec<&str> = chosen
        .iter()
        .map(|container| container.name.as_str())
        .collect();
    engine.remove(&names).await?;
    for name in names {
        println!("removed {name}");
    }
    Ok(())
}

/// Names vz sets inside itself: the configured environment cannot change them.
fn reserved_env_names() -> Vec<&'static str> {
    [
        USER_ENV,
        UID_ENV,
        GID_ENV,
        GROUP_ENV,
        HOME_ENV,
        GROUPS_ENV,
        DOCKER_HOST_ENV,
        SUDO_ENV,
        SHELL_ENV,
        CONTAINER_ENV,
        CONTAINER_PROFILE_ENV,
        HOOKS_CREATE_ENV,
        HOOKS_ATTACH_ENV,
        REPO_ENV,
    ]
    .into_iter()
    .chain(PASSTHROUGH_ENV)
    .collect()
}

/// Names and sources, aligned; never a value.
fn print_env(header: &str, environment: &env::Environment) {
    println!("# environment of {header}: names and sources, never values");
    let width = environment.keys().map(String::len).max().unwrap_or(0);
    for (name, var) in environment {
        println!("{name:width$}  {}", var.source);
    }
    println!(
        "# names vz sets itself, which these cannot change: {}",
        reserved_env_names().join(", ")
    );
}

/// Makes the top image of the chain unless the engine has it: builds each
/// image below it the engine lacks, bottom first, then the top; pulls the
/// top when it is a reference. A build pulls what it stands on itself.
async fn prepare_image(engine: &Engine, steps: &[ImageStep]) -> anyhow::Result<()> {
    let (top, below) = steps.split_last().expect("an image chain has a top");
    if engine.has_image(top.tag()).await {
        debug!("image {} is present", top.tag());
        return Ok(());
    }
    for step in below {
        if let ImageStep::Build(plan) = step {
            match engine.has_image(&plan.tag).await {
                true => debug!("image {} is present", plan.tag),
                false => engine.build(plan).await?,
            }
        }
    }
    match top {
        ImageStep::Pull(reference) => engine.pull(reference).await,
        ImageStep::Build(plan) => engine.build(plan).await,
    }
}

fn passthrough_env() -> Vec<(String, String)> {
    PASSTHROUGH_ENV
        .iter()
        .filter_map(|name| Some((name.to_string(), std::env::var(name).ok()?)))
        .collect()
}

/// Events and spans to stderr, filtered by `VZ_LOG`; stdout stays the container's.
fn init_tracing() {
    let filter =
        EnvFilter::try_from_env(LOG_ENV).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    #[test]
    fn reserved_env_names__hooks_and_repo__included() {
        let reserved = reserved_env_names();

        for name in ["VZ_HOOKS_CREATE", "VZ_HOOKS_ATTACH", "VZ_REPO"] {
            assert!(reserved.contains(&name), "{name}: {reserved:?}");
        }
    }
}
