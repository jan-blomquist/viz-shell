use std::io::Write;

use anyhow::{Context, bail};
use bollard::Docker;
use bollard::errors::Error as BollardError;
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    CreateImageOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
};
use futures_util::{StreamExt, TryStreamExt};
use log::{debug, info};

/// The container engine, reached through the Docker API.
pub struct Engine {
    docker: Docker,
}

impl Engine {
    /// Connects through `DOCKER_HOST`, or the default socket when it is unset.
    pub fn connect() -> anyhow::Result<Self> {
        let docker = Docker::connect_with_defaults().context("connecting to the docker engine")?;
        debug!("connected to the docker engine");
        Ok(Self { docker })
    }

    /// Runs `image` to completion, copies its output to `out`, removes the
    /// container, and returns the container's exit code.
    pub async fn run(&self, image: &str, out: &mut impl Write) -> anyhow::Result<i64> {
        self.pull_if_missing(image).await?;
        let body = ContainerCreateBody {
            image: Some(image.to_owned()),
            ..Default::default()
        };
        let id = self
            .docker
            .create_container(None, body)
            .await
            .context("creating the container")?
            .id;
        debug!("created container {id} from {image}");

        let outcome = self.start_and_follow(&id, out).await;
        self.docker
            .remove_container(
                &id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
            .context("removing the container")?;
        debug!("removed container {id}");
        outcome
    }

    async fn pull_if_missing(&self, image: &str) -> anyhow::Result<()> {
        if self.docker.inspect_image(image).await.is_ok() {
            debug!("image {image} is present");
            return Ok(());
        }
        info!("pulling {image}");
        let options = CreateImageOptionsBuilder::default()
            .from_image(image)
            .build();
        self.docker
            .create_image(Some(options), None, None)
            .try_collect::<Vec<_>>()
            .await
            .with_context(|| format!("pulling {image}"))?;
        Ok(())
    }

    async fn start_and_follow(&self, id: &str, out: &mut impl Write) -> anyhow::Result<i64> {
        self.docker
            .start_container(id, None)
            .await
            .context("starting the container")?;
        debug!("started container {id}");

        let options = LogsOptionsBuilder::default()
            .follow(true)
            .stdout(true)
            .stderr(true)
            .build();
        let mut logs = self.docker.logs(id, Some(options));
        while let Some(chunk) = logs.next().await {
            out.write_all(&chunk.context("reading container output")?.into_bytes())?;
        }

        let code = self.exit_code(id).await?;
        debug!("container {id} exited with {code}");
        Ok(code)
    }

    /// Bollard reports a non-zero exit as an error; here it is just the exit code.
    async fn exit_code(&self, id: &str) -> anyhow::Result<i64> {
        match self.docker.wait_container(id, None).try_next().await {
            Ok(Some(response)) => Ok(response.status_code),
            Ok(None) => bail!("container {id} ended without an exit status"),
            Err(BollardError::DockerContainerWaitError { code, .. }) => Ok(code),
            Err(error) => Err(error).context("waiting for the container"),
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "needs a docker engine"]
    async fn run__hello_world__exits_zero_and_prints_greeting() {
        let engine = Engine::connect().unwrap();
        let mut output = Vec::new();

        let exit_code = engine.run("hello-world:latest", &mut output).await.unwrap();

        assert_eq!(exit_code, 0);
        assert!(String::from_utf8_lossy(&output).contains("Hello from Docker!"));
    }
}
