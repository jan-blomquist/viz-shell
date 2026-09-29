use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use anyhow::{Context, bail, ensure};
use docker_wrapper::{DockerCommand, InspectCommand, PullCommand, RunCommand, ensure_docker};
use tracing::{debug, info, instrument};

use crate::build::BuildPlan;
use crate::constants::DOCKER_CLI;

/// The container engine, driven through the docker CLI. Only `detect`
/// makes one, so every engine has passed its checks.
pub struct Engine(());

impl Engine {
    /// Fails, naming the fix, unless the docker CLI is on `PATH`, recent
    /// enough, and reaches a running daemon.
    pub async fn detect() -> anyhow::Result<Self> {
        let info = match ensure_docker().await {
            Ok(info) => info,
            Err(docker_wrapper::Error::DockerNotFound) => bail!(
                "the {DOCKER_CLI} CLI is not on PATH; vz runs every engine operation through it. \
                 Install Docker: https://docs.docker.com/engine/install/"
            ),
            Err(error) => return Err(error).context(format!("checking the {DOCKER_CLI} CLI")),
        };
        if !info.daemon_running {
            bail!(
                "{} cannot reach the docker daemon; start it, or check DOCKER_HOST and `docker context ls`",
                info.binary_path
            );
        }
        debug!(
            "{} {}, daemon {}",
            info.binary_path,
            info.version.version,
            info.server_version
                .map(|version| version.version)
                .unwrap_or_default()
        );
        Ok(Self(()))
    }

    pub async fn has_image(&self, image: &str) -> bool {
        InspectCommand::new(image)
            .object_type("image")
            .execute()
            .await
            .is_ok()
    }

    #[instrument(skip(self))]
    pub async fn pull(&self, image: &str) -> anyhow::Result<()> {
        info!("pulling {image}");
        PullCommand::new(image)
            .execute()
            .await
            .with_context(|| format!("pulling {image}"))?;
        Ok(())
    }

    #[instrument(skip_all, fields(tag = %plan.tag))]
    pub async fn build(&self, plan: &BuildPlan) -> anyhow::Result<()> {
        info!("building {}", plan.tag);
        let status = attached(plan.command().build_command_args()).await?;
        ensure!(status.success(), "building {} failed: {status}", plan.tag);
        Ok(())
    }

    /// Runs the container on this terminal and returns its exit code.
    #[instrument(skip_all)]
    pub async fn run(&self, command: RunCommand) -> anyhow::Result<i32> {
        let status = attached(command.build_command_args()).await?;
        Ok(exit_code(status))
    }
}

/// Runs the docker CLI on this terminal: docker-wrapper's own `execute`
/// captures output, which hides build progress and cannot carry a TTY.
async fn attached(args: Vec<String>) -> anyhow::Result<ExitStatus> {
    debug!("{DOCKER_CLI} {}", args.join(" "));
    tokio::process::Command::new(DOCKER_CLI)
        .args(&args)
        .status()
        .await
        .with_context(|| format!("running {DOCKER_CLI}"))
}

/// A shell's convention: a process killed by a signal exits with 128 + signal.
fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or_default())
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    #[test]
    fn exit_code__exits_and_signals() {
        let cases = [
            (ExitStatus::from_raw(0), 0),
            (ExitStatus::from_raw(3 << 8), 3),
            (ExitStatus::from_raw(9), 137),
        ];
        for (status, expected) in cases {
            assert_eq!(exit_code(status), expected, "status: {status:?}");
        }
    }

    #[tokio::test]
    #[ignore = "needs a docker engine"]
    async fn run__hello_world__exits_zero() {
        let engine = Engine::detect().await.unwrap();
        engine.pull("hello-world:latest").await.unwrap();

        let run = RunCommand::new("hello-world:latest").remove();

        let exit_code = engine.run(run).await.unwrap();

        assert_eq!(exit_code, 0);
    }
}
