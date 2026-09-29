use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitStatus, Output};

use anyhow::{Context, bail, ensure};
use docker_wrapper::{
    DockerCommand, GenericCommand, InspectCommand, PsCommand, PullCommand, RmCommand, StartCommand,
    ensure_docker,
};
use tracing::{debug, info, instrument};

use crate::build::BuildPlan;
use crate::constants::{DOCKER_CLI, REPO_LABEL};
use crate::containers::{self, Container};

/// What `docker create` did.
#[derive(Debug, PartialEq)]
pub enum Created {
    Yes,
    /// Another container has the name: one created meanwhile.
    NameTaken,
}

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

    /// The current endpoint, as `docker context inspect` reports it: it
    /// follows `DOCKER_HOST` and `docker context use`.
    pub async fn docker_endpoint(&self) -> anyhow::Result<String> {
        let output = GenericCommand::new("context")
            .args(["inspect", "--format", "{{.Endpoints.docker.Host}}"])
            .execute()
            .await
            .context("asking docker for its endpoint")?;
        Ok(output.stdout.trim().to_owned())
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
        let status = attached(plan.command().build_command_args(), &[]).await?;
        ensure!(status.success(), "building {} failed: {status}", plan.tag);
        Ok(())
    }

    /// The containers of the repository at `repo`, running or not; every
    /// repository's with `None`. `docker ps` finds them by label, and
    /// `docker inspect` describes them: its JSON is docker's own.
    pub async fn containers(&self, repo: Option<&Path>) -> anyhow::Result<Vec<Container>> {
        let filter = match repo {
            Some(repo) => format!("label={REPO_LABEL}={}", repo.display()),
            None => format!("label={REPO_LABEL}"),
        };
        let ids = PsCommand::new()
            .all()
            .quiet()
            .filter(filter)
            .execute()
            .await
            .context("listing containers")?
            .container_ids();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let inspected = InspectCommand::new_multiple(ids)
            .object_type("container")
            .execute()
            .await;
        let json = match inspected {
            Ok(output) => output.stdout,
            // One removed since `docker ps` fails the call; the rest are there.
            Err(docker_wrapper::Error::CommandFailed { stdout, .. }) if !stdout.is_empty() => {
                stdout
            }
            Err(error) => return Err(error).context("inspecting containers"),
        };
        containers::parse_inspect(&json)
    }

    /// `docker create` with `args`; `env` as for `exec`. Spawned directly:
    /// docker-wrapper cannot hand the docker CLI an environment.
    pub async fn create(
        &self,
        args: Vec<String>,
        env: &[(String, String)],
    ) -> anyhow::Result<Created> {
        debug!("{DOCKER_CLI} {}", args.join(" "));
        let output = captured(&args, env).await?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() && stderr.contains("is already in use") {
            return Ok(Created::NameTaken);
        }
        checked(output, "creating the container")?;
        Ok(Created::Yes)
    }

    pub async fn start(&self, name: &str) -> anyhow::Result<()> {
        StartCommand::new(name)
            .execute()
            .await
            .with_context(|| format!("starting {name}"))?;
        Ok(())
    }

    /// Starts the container on this terminal and returns its exit code.
    #[instrument(skip(self))]
    pub async fn start_attached(&self, name: &str) -> anyhow::Result<i32> {
        let args = StartCommand::new(name)
            .attach()
            .interactive()
            .build_command_args();
        Ok(exit_code(attached(args, &[]).await?))
    }

    /// Runs `docker exec` with `args` on this terminal and returns the
    /// command's exit code. `env` goes into the docker CLI's own environment,
    /// where `--env NAME` takes its values from.
    #[instrument(skip_all)]
    pub async fn exec(&self, args: Vec<String>, env: &[(String, String)]) -> anyhow::Result<i32> {
        Ok(exit_code(attached(args, env).await?))
    }

    /// Stops and removes the containers, and the shells attached to them.
    pub async fn remove(&self, names: &[&str]) -> anyhow::Result<()> {
        RmCommand::new_multiple(names.to_vec())
            .force()
            .execute()
            .await
            .context("removing containers")?;
        Ok(())
    }
}

/// Runs the docker CLI, capturing its output.
async fn captured(
    args: &[impl AsRef<std::ffi::OsStr>],
    env: &[(String, String)],
) -> anyhow::Result<Output> {
    tokio::process::Command::new(DOCKER_CLI)
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .output()
        .await
        .with_context(|| format!("running {DOCKER_CLI}"))
}

/// The output's stdout, or its stderr as the error of `doing`.
fn checked(output: Output, doing: &str) -> anyhow::Result<String> {
    ensure!(
        output.status.success(),
        "{doing}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Runs the docker CLI on this terminal: docker-wrapper's own `execute`
/// captures output, which hides build progress and cannot carry a TTY.
async fn attached(args: Vec<String>, env: &[(String, String)]) -> anyhow::Result<ExitStatus> {
    debug!("{DOCKER_CLI} {}", args.join(" "));
    tokio::process::Command::new(DOCKER_CLI)
        .args(&args)
        .envs(env.iter().map(|(name, value)| (name, value)))
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
    use docker_wrapper::RunCommand;

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
    async fn exec__hello_world__exits_zero() {
        let engine = Engine::detect().await.unwrap();
        engine.pull("hello-world:latest").await.unwrap();

        let run = RunCommand::new("hello-world:latest").remove();

        let exit_code = engine.exec(run.build_command_args(), &[]).await.unwrap();

        assert_eq!(exit_code, 0);
    }
}
