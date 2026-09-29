mod build;
mod cli;
mod config;
mod constants;
mod engine;
mod entrypoint;
mod mounts;
mod repo;
mod session;
mod share;
mod state;
mod user;

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, ensure};
use clap::Parser;
use tracing::debug;
use tracing_subscriber::EnvFilter;

use crate::build::BuildPlan;
use crate::cli::{Cli, Internal};
use crate::config::{ImageSource, RepoConfig};
use crate::constants::{
    DEFAULT_LOG_FILTER, DEFAULT_STATE_DIR, ENTRYPOINT_PATH, LOG_ENV, PASSTHROUGH_ENV,
    REPO_CONFIG_FILE,
};
use crate::engine::Engine;
use crate::session::Session;
use crate::share::DockerSocket;
use crate::user::User;

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.internal {
        // Before any runtime starts: the entrypoint changes user and execs,
        // which wants a single-threaded process.
        Some(Internal::Entrypoint { command }) => entrypoint::run(&command),
        None => {
            let runtime = tokio::runtime::Runtime::new().context("starting the async runtime")?;
            let exit_code = runtime.block_on(launch(&cli))?;
            std::process::exit(exit_code);
        }
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
    let repo_root = repo::root()?;
    let config_file = match &cli.config_file {
        Some(file) => {
            std::path::absolute(file).with_context(|| format!("resolving {}", file.display()))?
        }
        None => repo_root.join(REPO_CONFIG_FILE),
    };
    let config = RepoConfig::load(&config_file)?
        .effective(cli.profile.as_deref())
        .with_context(|| format!("in {}", config_file.display()))?;
    if cli.show_effective_config {
        let profile = cli
            .profile
            .as_deref()
            .map(|name| format!(", profile {name}"))
            .unwrap_or_default();
        println!(
            "# effective configuration of {}{profile}",
            config_file.display()
        );
        print!("{}", config.to_yaml()?);
        return Ok(0);
    }
    debug!("effective configuration: {config:?}");
    // Paths in the configuration are relative to its folder.
    let config_dir = config_file.parent().unwrap_or(&repo_root).to_owned();
    let user = User::of_host()?;
    debug!("host user: {user:?}");

    let state_dir = match &config.state_dir {
        Some(dir) => config::resolve_host_path(dir, &config_dir, &user.home),
        None => repo_root.join(DEFAULT_STATE_DIR),
    };
    let state = state::plan(&config.state, &user.home, &state_dir, &repo_root)?;
    let state_targets: Vec<_> = state.iter().map(|mount| mount.target.clone()).collect();
    let mounts = mounts::plan(&config.mounts, &user.home, &state_targets)?;
    mounts::check_sources_exist(&mounts)?;

    let vz_binary = std::env::current_exe().context("locating the vz binary")?;
    let inside_vz = Path::new(ENTRYPOINT_PATH).exists();
    session::check_binary_reachable(inside_vz, &vz_binary, &repo_root)?;

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
        command: &cli.command,
        tty: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
    };
    engine.run(session.run_command()).await
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
