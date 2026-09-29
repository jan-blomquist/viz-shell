//! Sharing the host's docker daemon: its socket at the same path inside, and
//! its group, which the user inside joins, so docker works as on the host.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use anyhow::{Context, bail};
use nix::unistd::{Gid, Group};

use crate::constants::{DOCKER_GROUP_NAME, DOCKER_HOST_ENV, GROUPS_ENV, UNIX_SOCKET_SCHEME};

#[derive(Debug, Clone, PartialEq)]
pub struct DockerSocket {
    pub path: PathBuf,
    /// The socket's group on the host, which the user inside joins.
    pub group: String,
    pub gid: u32,
}

impl DockerSocket {
    /// The socket behind the engine's current endpoint, which follows
    /// `DOCKER_HOST` and `docker context use`.
    pub fn locate(endpoint: &str) -> anyhow::Result<Self> {
        let path = unix_socket_path(endpoint)?;
        let gid = std::fs::metadata(&path)
            .with_context(|| format!("reading the docker socket {}", path.display()))?
            .gid();
        let group = Group::from_gid(Gid::from_raw(gid))
            .ok()
            .flatten()
            .map_or_else(|| DOCKER_GROUP_NAME.to_owned(), |group| group.name);
        Ok(Self { path, group, gid })
    }

    /// Inside, docker finds the socket through `DOCKER_HOST`, and the
    /// entrypoint joins the group named in `VZ_GROUPS`.
    pub fn env(&self) -> [(&'static str, String); 2] {
        [
            (
                DOCKER_HOST_ENV,
                format!("{UNIX_SOCKET_SCHEME}{}", self.path.display()),
            ),
            (GROUPS_ENV, format!("{}:{}", self.group, self.gid)),
        ]
    }
}

fn unix_socket_path(endpoint: &str) -> anyhow::Result<PathBuf> {
    match endpoint.trim().strip_prefix(UNIX_SOCKET_SCHEME) {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => bail!(
            "share.docker needs a local unix socket, but the docker endpoint is `{}`; \
             point DOCKER_HOST or `docker context use` at a unix:// socket",
            endpoint.trim()
        ),
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    #[test]
    fn unix_socket_path__endpoints() {
        let accepted = [
            ("unix:///var/run/docker.sock", "/var/run/docker.sock"),
            (
                "unix:///run/user/1000/docker.sock\n",
                "/run/user/1000/docker.sock",
            ),
        ];
        for (endpoint, expected) in accepted {
            assert_eq!(
                unix_socket_path(endpoint).unwrap(),
                PathBuf::from(expected),
                "endpoint: {endpoint:?}"
            );
        }
    }

    #[test]
    fn unix_socket_path__remote_endpoint__is_refused_naming_it() {
        let refused = [
            "tcp://10.0.0.1:2375",
            "ssh://sally@build-host",
            "unix://",
            "",
        ];
        for endpoint in refused {
            let error = unix_socket_path(endpoint).unwrap_err().to_string();

            assert!(error.contains(&format!("`{endpoint}`")), "{error}");
        }
    }

    #[test]
    fn env__socket__docker_host_and_group() {
        let socket = DockerSocket {
            path: PathBuf::from("/var/run/docker.sock"),
            group: "docker".to_owned(),
            gid: 969,
        };

        let env = socket.env();

        assert_eq!(
            env,
            [
                ("DOCKER_HOST", "unix:///var/run/docker.sock".to_owned()),
                ("VZ_GROUPS", "docker:969".to_owned()),
            ]
        );
    }
}
