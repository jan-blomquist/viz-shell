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

use crate::build::BuildPlan;
use crate::cli::{Action, Cli};
use crate::config::{Config, ImageSource, Layer};
use crate::constants::{
    DEFAULT_LOG_FILTER, DEFAULT_STATE_DIR, DOCKER_HOST_ENV, ENTRYPOINT_PATH, GID_ENV,
    GLOBAL_CONFIG_DIR, GLOBAL_CONFIG_FILE, GROUP_ENV, GROUPS_ENV, HOME_ENV, LOG_ENV,
    MOUNTINFO_FILE, PASSTHROUGH_ENV, REPO_CONFIG_FILES, SHELL_ENV, SUDO_ENV, UID_ENV, USER_ENV,
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
    config: Config,
}

impl Loaded {
    /// `# global …, repo …`: the files read.
    fn files_line(&self) -> String {
        let files: Vec<String> = [("global", &self.global_file), ("repo", &self.repo_file)]
            .into_iter()
            .filter_map(|(origin, file)| Some(format!("{origin} {}", file.as_ref()?.display())))
            .collect();
        files.join(", ")
    }
}

/// Reads the global configuration and the repository's, either optional
/// but not both. `-c` names the repository's.
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
    let read = |file: &Option<PathBuf>| {
        file.as_deref()
            .map(|file| Layer::load(file, &user.home, &repo_root))
            .transpose()
    };
    let config = Config::new(read(&global_file)?, read(&repo_file)?)?;
    Ok(Loaded {
        repo_root,
        user,
        global_file,
        repo_file,
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
        println!("No profiles yet: add them under `profiles:` in either file.");
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
    let files = [("global", &loaded.global_file), ("repo", &loaded.repo_file)];
    let rows = files.into_iter().filter_map(|(origin, file)| {
        let file = file.as_ref()?;
        Some([origin.to_owned(), config::tilde(file, &loaded.user.home)])
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
        print!("{}", config.to_yaml()?);
        return Ok(0);
    }
    debug!("effective configuration: {config:?}");
    let Loaded {
        repo_root,
        user,
        global_file,
        repo_file,
        ..
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
    let state_targets: Vec<_> = state.iter().map(|mount| mount.target.clone()).collect();
    let mounts = mounts::plan(&config.mounts, &user.home, &state_targets)?;
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
    let show_banner = |container: &str, image: &str, state_text: &str| {
        if !(config.banner && command.is_empty() && tty) {
            return;
        }
        let branch = repo::branch(&repo_root);
        let config_files: Vec<&Path> = [&global_file, &repo_file]
            .into_iter()
            .flatten()
            .map(PathBuf::as_path)
            .collect();
        let facts = banner::Session {
            user: &user.name,
            container,
            state: state_text,
            home: &user.home,
            repo_root: &repo_root,
            branch: branch.as_deref(),
            config_files: &config_files,
            profile,
            image,
            shell: config.shell.as_deref(),
            sudo: config.privileges.sudo,
            docker: docker.as_ref().map(|socket| socket.path.as_path()),
            host_network: config.share.host_network,
            mounts: &mounts,
            state_paths: state.len(),
            state_dir: &state_dir,
            env_vars: environment.len(),
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
        show_banner(&container.name, &container.image, "attached");
        return engine.exec(enter(&container.name).args(), &values).await;
    }

    state::create_sources(&state)?;
    let image = prepare_image(&engine, &config.image, &config_dir, &repo_root, &user).await?;
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
        show_banner(&name, &image, "new, kept on exit");
        engine.exec(enter(&name).args(), &values).await
    } else {
        show_banner(&name, &image, "new, removed on exit");
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

/// Pulls or builds the image unless the engine already has it, and returns
/// its reference.
async fn prepare_image(
    engine: &Engine,
    source: &ImageSource,
    config_dir: &Path,
    repo_root: &Path,
    user: &User,
) -> anyhow::Result<String> {
    let (image, plan) = match source {
        ImageSource::Reference(reference) => (config::with_default_tag(reference), None),
        ImageSource::Build(spec) => {
            let plan = BuildPlan::load(spec, config_dir, &repo::dir_name(repo_root), user)?;
            (plan.tag.clone(), Some(plan))
        }
    };
    if engine.has_image(&image).await {
        debug!("image {image} is present");
        return Ok(image);
    }
    match plan {
        Some(plan) => engine.build(&plan).await?,
        None => engine.pull(&image).await?,
    }
    Ok(image)
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
