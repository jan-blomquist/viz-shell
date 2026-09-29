//! Plans an image build from a Dockerfile. The tag carries a hash of the
//! Dockerfile and its args, so an edit anywhere else in `vz.yml` never
//! rebuilds, and neither does a change to a file the Dockerfile copies.
//!
//! A Dockerfile that declares `ARG VZ_UID` (or any of the user's build args)
//! gets the host user's value, so it can bake the user into the image; the
//! values join the hash, so such images are built per user.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use docker_wrapper::BuildCommand;
use sha2::{Digest, Sha256};

use crate::config::BuildSpec;
use crate::constants::{BUILT_IMAGE_PREFIX, CONTENT_HASH_LEN, FALLBACK_IMAGE_NAME};
use crate::user::User;

#[derive(Debug)]
pub struct BuildPlan {
    pub tag: String,
    dockerfile: PathBuf,
    context: PathBuf,
    args: BTreeMap<String, String>,
}

impl BuildPlan {
    /// Resolves the spec's paths against the folder of its `vz.yml`, reads the
    /// Dockerfile, and adds the user's build args it declares; `vz.yml` args win.
    pub fn load(
        spec: &BuildSpec,
        config_dir: &Path,
        repo_dir_name: &str,
        user: &User,
    ) -> anyhow::Result<Self> {
        let dockerfile = config_dir.join(&spec.dockerfile);
        let dockerfile_text = std::fs::read_to_string(&dockerfile)
            .with_context(|| format!("reading {}", dockerfile.display()))?;
        let mut args = identity_args(&dockerfile_text, user);
        args.extend(spec.args.clone());
        let resolved = BuildSpec {
            dockerfile,
            context: config_dir.join(&spec.context),
            args,
        };
        Ok(Self::new(&resolved, repo_dir_name, &dockerfile_text))
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

/// The user's build args that the Dockerfile declares.
fn identity_args(dockerfile_text: &str, user: &User) -> BTreeMap<String, String> {
    let declared = declared_args(dockerfile_text);
    user.build_args()
        .into_iter()
        .filter(|(name, _)| declared.contains(name))
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

/// Names from `ARG` instructions: `ARG A`, `ARG A=default`, `ARG A B`.
fn declared_args(dockerfile_text: &str) -> BTreeSet<&str> {
    dockerfile_text
        .lines()
        .filter_map(|line| {
            let (instruction, rest) = line.trim_start().split_once(char::is_whitespace)?;
            instruction.eq_ignore_ascii_case("ARG").then_some(rest)
        })
        .flat_map(str::split_whitespace)
        .map(|arg| arg.split('=').next().unwrap_or_default())
        .collect()
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
    let mut hex = hex::encode(hasher.finalize());
    hex.truncate(CONTENT_HASH_LEN);
    hex
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

    fn sally() -> User {
        User {
            name: "sally".to_owned(),
            uid: 1000,
            gid: 1000,
            group: "sally".to_owned(),
            home: PathBuf::from("/home/sally"),
        }
    }

    #[test]
    fn declared_args__arg_forms() {
        let dockerfile = "FROM alpine\n\
                          ARG PLAIN\n\
                          ARG WITH_DEFAULT=1\n\
                          arg lower\n\
                          ARG FIRST SECOND=2\n\
                          # ARG COMMENTED\n\
                          RUN echo ARG NOT_AN_ARG\n";

        let declared = declared_args(dockerfile);

        let expected = BTreeSet::from(["FIRST", "PLAIN", "SECOND", "WITH_DEFAULT", "lower"]);
        assert_eq!(declared, expected);
    }

    #[test]
    fn identity_args__nothing_declared__is_empty() {
        let args = identity_args("FROM alpine\nARG BASE\n", &sally());

        assert!(args.is_empty(), "{args:?}");
    }

    #[test]
    fn identity_args__some_declared__passes_only_those() {
        let dockerfile = "FROM alpine\nARG VZ_USER VZ_UID\nARG VZ_HOME=/home/nobody\n";

        let args = identity_args(dockerfile, &sally());

        let expected = BTreeMap::from([
            ("VZ_HOME".to_owned(), "/home/sally".to_owned()),
            ("VZ_UID".to_owned(), "1000".to_owned()),
            ("VZ_USER".to_owned(), "sally".to_owned()),
        ]);
        assert_eq!(args, expected);
    }

    #[test]
    fn load__paths_relative_to_config_dir__vz_yml_args_win_over_identity() {
        let config_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            config_dir.path().join("Dockerfile"),
            "FROM alpine\nARG VZ_UID\n",
        )
        .unwrap();
        let spec = BuildSpec {
            dockerfile: PathBuf::from("Dockerfile"),
            context: PathBuf::from("."),
            args: args(&[("VZ_UID", "4242")]),
        };

        let plan = BuildPlan::load(&spec, config_dir.path(), "app", &sally()).unwrap();

        assert_eq!(plan.dockerfile, config_dir.path().join("Dockerfile"));
        assert_eq!(plan.context, config_dir.path().join("."));
        assert_eq!(plan.args, args(&[("VZ_UID", "4242")]));
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
            args: args(&[("BASE", "alpine"), ("USER", "sally")]),
        };
        let plan = BuildPlan::new(&spec, "vz", DOCKERFILE);

        let cli_args = plan.command().build_command_args();

        // Build args come from a HashMap in docker-wrapper: pairs, not positions.
        let has = |flag: &str, value: &str| cli_args.windows(2).any(|w| w == [flag, value]);
        assert_eq!(cli_args.first().map(String::as_str), Some("build"));
        assert!(has("--tag", &plan.tag), "{cli_args:?}");
        assert!(has("--file", "shell/Dockerfile"), "{cli_args:?}");
        assert!(has("--build-arg", "BASE=alpine"), "{cli_args:?}");
        assert!(has("--build-arg", "USER=sally"), "{cli_args:?}");
        assert_eq!(cli_args.last().map(String::as_str), Some("ctx"));
    }
}
