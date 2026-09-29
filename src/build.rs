//! Plans an image build from a Dockerfile. The tag carries a hash of the
//! Dockerfile and its args, so an edit anywhere else in `vz.yml` never
//! rebuilds, and neither does a change to a file the Dockerfile copies.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;
use docker_wrapper::BuildCommand;
use sha2::{Digest, Sha256};

use crate::config::BuildSpec;
use crate::constants::{BUILT_IMAGE_PREFIX, CONTENT_HASH_LEN, FALLBACK_IMAGE_NAME};

#[derive(Debug)]
pub struct BuildPlan {
    pub tag: String,
    dockerfile: PathBuf,
    context: PathBuf,
    args: BTreeMap<String, String>,
}

impl BuildPlan {
    pub fn load(spec: &BuildSpec, repo_dir_name: &str) -> anyhow::Result<Self> {
        let dockerfile_text = std::fs::read_to_string(&spec.dockerfile)
            .with_context(|| format!("reading {}", spec.dockerfile.display()))?;
        Ok(Self::new(spec, repo_dir_name, &dockerfile_text))
    }

    fn new(spec: &BuildSpec, repo_dir_name: &str, dockerfile_text: &str) -> Self {
        Self {
            tag: image_tag(repo_dir_name, dockerfile_text, &spec.args),
            dockerfile: spec.dockerfile.clone(),
            context: spec.context.clone(),
            args: spec.args.clone(),
        }
    }

    /// `docker build`, which sends the daemon only the files the build uses
    /// and honours `.dockerignore`.
    pub fn command(&self) -> BuildCommand {
        self.args.iter().fold(
            BuildCommand::new(self.context.to_string_lossy())
                .tag(&self.tag)
                .file(&self.dockerfile),
            |command, (name, value)| command.build_arg(name, value),
        )
    }
}

/// `vz-<repository directory>:<content hash>`.
fn image_tag(
    repo_dir_name: &str,
    dockerfile_text: &str,
    args: &BTreeMap<String, String>,
) -> String {
    format!(
        "{BUILT_IMAGE_PREFIX}{}:{}",
        image_name(repo_dir_name),
        content_hash(dockerfile_text, args)
    )
}

/// An image name allows lowercase letters, digits and inner separators;
/// everything else becomes `-`.
fn image_name(repo_dir_name: &str) -> String {
    let name: String = repo_dir_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        FALLBACK_IMAGE_NAME.to_owned()
    } else {
        name.to_owned()
    }
}

fn content_hash(dockerfile_text: &str, args: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(dockerfile_text);
    for (name, value) in args {
        // NUL separators keep `A=BC` and `AB=C` apart.
        hasher.update([0]);
        hasher.update(name);
        hasher.update([0]);
        hasher.update(value);
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    hex[..CONTENT_HASH_LEN].to_owned()
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use docker_wrapper::DockerCommand;

    use super::*;

    const DOCKERFILE: &str = "FROM alpine:3\n";

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn image_tag__any_input__is_prefixed_name_and_hash() {
        let tag = image_tag("viz-shell", DOCKERFILE, &args(&[]));

        let (name, hash) = tag.split_once(':').unwrap();
        assert_eq!(name, "vz-viz-shell");
        assert_eq!(hash.len(), CONTENT_HASH_LEN);
    }

    #[test]
    fn image_tag__same_input__same_tag() {
        let first = image_tag("vz", DOCKERFILE, &args(&[("A", "1")]));

        let second = image_tag("vz", DOCKERFILE, &args(&[("A", "1")]));

        assert_eq!(first, second);
    }

    #[test]
    fn image_tag__dockerfile_edit__changes_tag() {
        let before = image_tag("vz", DOCKERFILE, &args(&[]));

        let after = image_tag("vz", "FROM alpine:3\nRUN true\n", &args(&[]));

        assert_ne!(before, after);
    }

    #[test]
    fn image_tag__arg_change__changes_tag() {
        let before = image_tag("vz", DOCKERFILE, &args(&[("A", "1")]));

        let after = image_tag("vz", DOCKERFILE, &args(&[("A", "2")]));

        assert_ne!(before, after);
    }

    #[test]
    fn image_tag__arg_boundary_moved__changes_tag() {
        let before = image_tag("vz", DOCKERFILE, &args(&[("A", "BC")]));

        let after = image_tag("vz", DOCKERFILE, &args(&[("AB", "C")]));

        assert_ne!(before, after);
    }

    #[test]
    fn image_name__directory_names() {
        let cases = [
            ("vz", "vz"),
            ("viz-shell", "viz-shell"),
            ("My Repo", "my-repo"),
            ("_private_", "private"),
            ("über", "ber"),
            ("...", "repo"),
        ];
        for (dir_name, expected) in cases {
            assert_eq!(image_name(dir_name), expected, "dir_name: {dir_name}");
        }
    }

    #[test]
    fn command__dockerfile_context_and_args__passes_each_to_docker_build() {
        let spec = BuildSpec {
            dockerfile: PathBuf::from("shell/Dockerfile"),
            context: PathBuf::from("ctx"),
            args: args(&[("BASE", "alpine"), ("USER", "jan")]),
        };
        let plan = BuildPlan::new(&spec, "vz", DOCKERFILE);

        let cli_args = plan.command().build_command_args();

        // Build args come from a HashMap in docker-wrapper: pairs, not positions.
        let has = |flag: &str, value: &str| cli_args.windows(2).any(|w| w == [flag, value]);
        assert_eq!(cli_args.first().map(String::as_str), Some("build"));
        assert!(has("--tag", &plan.tag), "{cli_args:?}");
        assert!(has("--file", "shell/Dockerfile"), "{cli_args:?}");
        assert!(has("--build-arg", "BASE=alpine"), "{cli_args:?}");
        assert!(has("--build-arg", "USER=jan"), "{cli_args:?}");
        assert_eq!(cli_args.last().map(String::as_str), Some("ctx"));
    }
}
