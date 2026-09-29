use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

use crate::constants::{DEFAULT_BUILD_CONTEXT, DEFAULT_TAG};

/// The repository configuration, `vz.yml`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub image: ImageSource,
}

/// Where the image comes from: a reference to pull, or a Dockerfile to build.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ImageSource {
    Reference(String),
    Build(BuildSpec),
}

/// Paths are relative to the repository root.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSpec {
    pub dockerfile: PathBuf,
    #[serde(default = "default_build_context")]
    pub context: PathBuf,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

fn default_build_context() -> PathBuf {
    PathBuf::from(DEFAULT_BUILD_CONTEXT)
}

impl RepoConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        serde_saphyr::from_str(text).context("invalid repository configuration")
    }
}

/// The reference as the engine expects it: without a tag or digest it gets
/// the default tag, because a pull without one fetches every tag.
pub fn with_default_tag(reference: &str) -> String {
    let name = reference.rsplit('/').next().unwrap_or_default();
    if name.contains(':') || name.contains('@') {
        reference.to_owned()
    } else {
        format!("{reference}:{DEFAULT_TAG}")
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    #[test]
    fn parse__image_reference__reads_reference() {
        let text = "image: hello-world\n";

        let config = RepoConfig::parse(text).unwrap();

        assert_eq!(
            config.image,
            ImageSource::Reference("hello-world".to_owned())
        );
    }

    #[test]
    fn parse__dockerfile_only__defaults_context_and_args() {
        let text = "image:\n  dockerfile: Dockerfile\n";

        let config = RepoConfig::parse(text).unwrap();

        let expected = BuildSpec {
            dockerfile: PathBuf::from("Dockerfile"),
            context: PathBuf::from("."),
            args: BTreeMap::new(),
        };
        assert_eq!(config.image, ImageSource::Build(expected));
    }

    #[test]
    fn parse__dockerfile_with_context_and_args__reads_all() {
        let text =
            "image:\n  dockerfile: shell/Dockerfile\n  context: shell\n  args: { BASE: alpine }\n";

        let config = RepoConfig::parse(text).unwrap();

        let expected = BuildSpec {
            dockerfile: PathBuf::from("shell/Dockerfile"),
            context: PathBuf::from("shell"),
            args: BTreeMap::from([("BASE".to_owned(), "alpine".to_owned())]),
        };
        assert_eq!(config.image, ImageSource::Build(expected));
    }

    #[test]
    fn parse__unknown_key__is_refused_naming_the_key() {
        let text = "image: hello-world\nimgae: typo\n";

        let error = RepoConfig::parse(text).unwrap_err();

        assert!(format!("{error:#}").contains("imgae"), "{error:#}");
    }

    #[test]
    fn parse__unknown_key_in_build__is_refused() {
        let text = "image:\n  dockerfile: Dockerfile\n  dockerfle: typo\n";

        let result = RepoConfig::parse(text);

        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn with_default_tag__tag_and_digest_rules() {
        let cases = [
            ("hello-world", "hello-world:latest"),
            ("hello-world:linux", "hello-world:linux"),
            ("ghcr.io/org/base:1", "ghcr.io/org/base:1"),
            ("localhost:5000/base", "localhost:5000/base:latest"),
            ("alpine@sha256:abc", "alpine@sha256:abc"),
        ];
        for (reference, expected) in cases {
            assert_eq!(
                with_default_tag(reference),
                expected,
                "reference: {reference}"
            );
        }
    }
}
