mod banner;
mod build;
mod cli;
mod config;
mod constants;
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

use anyhow::{Context, ensure};
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
use crate::engine::Engine;
use crate::env::CliEnv;
use crate::session::Session;
use crate::share::DockerSocket;
use crate::user::User;

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match &cli.action {
        // Before any runtime starts: the entrypoint changes user and execs,
        // which wants a single-threaded process.
        Some(Action::Entrypoint { command }) => entrypoint::run(command),
        Some(Action::Profiles) => print_profiles(&load_with(cli.config_file.as_deref())?),
        None => {
            let runtime = tokio::runtime::Runtime::new().context("starting the async runtime")?;
            let exit_code = runtime.block_on(launch(&cli))?;
            std::process::exit(exit_code);
        }
    }
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
    let global_path = global_config_file(&user.home);
    match config::scaffold_global(&global_path) {
        Ok(true) => info!(
            "wrote the default global configuration to {}: edit it to taste",
            global_path.display()
        ),
        Ok(false) => {}
        Err(error) => warn!("no global configuration: {error:#}"),
    }
    let global_file = Some(global_path).filter(|file| file.is_file());
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
        global_config_file(&user.home).display()
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
fn global_config_file(home: &Path) -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from);
    config_home.join(GLOBAL_CONFIG_DIR).join(GLOBAL_CONFIG_FILE)
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

/// Starts the repository's container and runs the shell, or `command`, in it
/// as the host user; returns the exit code.
async fn launch(cli: &Cli) -> anyhow::Result<i32> {
    ensure!(
        cfg!(target_env = "musl"),
        "vz mounts itself into the container, so it must be a static musl build: \
         cargo build --target x86_64-unknown-linux-musl"
    );
    let loaded = load_with(cli.config_file.as_deref())?;
    let config = loaded
        .config
        .effective(cli.profile.as_deref())
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
    state::create_sources(&state)?;
    let image = prepare_image(&engine, &config.image, &config_dir, &repo_root, &user).await?;

    let workdir = std::env::current_dir().context("reading the current directory")?;
    let session = Session {
        image: &image,
        repo_root: &repo_root,
        workdir: &workdir,
        vz_binary: &vz_binary,
        user: &user,
        state: &state,
        mounts: &mounts,
        docker: docker.as_ref(),
        passthrough: &passthrough_env(),
        env_names: &environment.keys().cloned().collect::<Vec<_>>(),
        command: &cli.command,
        tty: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        sudo: config.privileges.sudo,
        host_network: config.share.host_network,
        shell: config.shell.as_deref(),
    };
    if config.banner && cli.command.is_empty() && session.tty {
        let branch = repo::branch(&repo_root);
        let config_files: Vec<&Path> = [&global_file, &repo_file]
            .into_iter()
            .flatten()
            .map(PathBuf::as_path)
            .collect();
        let facts = banner::Session {
            user: &user.name,
            home: &user.home,
            repo_root: &repo_root,
            branch: branch.as_deref(),
            config_files: &config_files,
            profile: cli.profile.as_deref(),
            image: &image,
            shell: config.shell.as_deref(),
            sudo: config.privileges.sudo,
            docker: docker.as_ref().map(|socket| socket.path.as_path()),
            host_network: config.share.host_network,
            mounts: &mounts,
            state_paths: state.len(),
            state_dir: &state_dir,
            env_vars: environment.len(),
        };
        let color = std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
        print!(
            "{}",
            banner::render(&banner::title(&facts), &banner::facts(&facts), color)
        );
    }
    let values: Vec<(String, String)> = environment
        .into_iter()
        .map(|(name, var)| (name, var.value))
        .collect();
    engine.run(session.run_args(), &values).await
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
