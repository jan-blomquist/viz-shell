use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// The repository configuration, `vz.yml`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub image: String,
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

    /// The image as the engine expects it: a reference without a tag or
    /// digest gets `:latest`, because a pull without one fetches every tag.
    pub fn image_reference(&self) -> String {
        let name = self.image.rsplit('/').next().unwrap_or_default();
        if name.contains(':') || name.contains('@') {
            self.image.clone()
        } else {
            format!("{}:latest", self.image)
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    fn with_image(image: &str) -> RepoConfig {
        RepoConfig {
            image: image.to_owned(),
        }
    }

    #[test]
    fn parse__minimal_config__reads_image() {
        let text = "image: hello-world\n";

        let config = RepoConfig::parse(text).unwrap();

        assert_eq!(config, with_image("hello-world"));
    }

    #[test]
    fn parse__unknown_key__is_refused_naming_the_key() {
        let text = "image: hello-world\nimgae: typo\n";

        let error = RepoConfig::parse(text).unwrap_err();

        assert!(format!("{error:#}").contains("imgae"), "{error:#}");
    }

    #[test]
    fn image_reference__tag_and_digest_rules() {
        let cases = [
            ("hello-world", "hello-world:latest"),
            ("hello-world:linux", "hello-world:linux"),
            ("ghcr.io/org/base:1", "ghcr.io/org/base:1"),
            ("localhost:5000/base", "localhost:5000/base:latest"),
            ("alpine@sha256:abc", "alpine@sha256:abc"),
        ];
        for (image, expected) in cases {
            assert_eq!(
                with_image(image).image_reference(),
                expected,
                "image: {image}"
            );
        }
    }
}
