mod build;
mod config;
mod constants;
mod engine;

use std::io::IsTerminal;
use std::path::Path;

use anyhow::Context;
use tracing::debug;
use tracing_subscriber::EnvFilter;

use crate::build::BuildPlan;
use crate::config::{ImageSource, RepoConfig};
use crate::constants::{DEFAULT_LOG_FILTER, LOG_ENV, REPO_CONFIG_FILE};
use crate::engine::Engine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = RepoConfig::load(Path::new(REPO_CONFIG_FILE))?;
    debug!("read {REPO_CONFIG_FILE}: {config:?}");

    let engine = Engine::detect().await?;
    let image = prepare_image(&engine, &config.image).await?;
    let exit_code = engine.run(&image).await?;
    std::process::exit(exit_code);
}

/// Pulls or builds the image unless the engine already has it, and returns
/// its reference.
async fn prepare_image(engine: &Engine, source: &ImageSource) -> anyhow::Result<String> {
    let (image, plan) = match source {
        ImageSource::Reference(reference) => (config::with_default_tag(reference), None),
        ImageSource::Build(spec) => {
            let plan = BuildPlan::load(spec, &repo_dir_name()?)?;
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

fn repo_dir_name() -> anyhow::Result<String> {
    let dir = std::env::current_dir().context("reading the current directory")?;
    Ok(dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default())
}
