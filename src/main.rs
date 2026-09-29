mod config;
mod constants;
mod engine;

use std::path::Path;

use log::debug;

use crate::config::RepoConfig;
use crate::constants::{DEFAULT_LOG_FILTER, LOG_ENV, REPO_CONFIG_FILE};
use crate::engine::Engine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::new().filter_or(LOG_ENV, DEFAULT_LOG_FILTER))
        .init();

    let config = RepoConfig::load(Path::new(REPO_CONFIG_FILE))?;
    debug!("read {REPO_CONFIG_FILE}: {config:?}");

    let engine = Engine::connect()?;
    let exit_code = engine
        .run(&config.image_reference(), &mut std::io::stdout())
        .await?;
    std::process::exit(exit_code as i32);
}
