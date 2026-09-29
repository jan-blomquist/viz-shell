use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use serde::Deserialize;

use crate::constants::{DEFAULT_BUILD_CONTEXT, DEFAULT_TAG, HOME_PREFIX};

/// The repository configuration, `vz.yml`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub image: ImageSource,
    /// Where state is kept: relative to this file's folder, `~/…` or
    /// absolute. Unset: `.vz_state` at the git root.
    pub state_dir: Option<String>,
    #[serde(default)]
    pub state: Vec<StateEntry>,
    #[serde(default)]
    pub mounts: Vec<MountEntry>,
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

/// A path inside the container whose contents outlive it, stored under the
/// repository's cache folder. Written as a bare path for a folder, or in full.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "StateEntryForm")]
pub struct StateEntry {
    /// `~/…` or absolute.
    pub path: String,
    pub kind: StateKind,
    /// A file's content when vz creates it.
    pub init: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StateKind {
    #[default]
    Dir,
    File,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StateEntryForm {
    Path(String),
    Full(FullStateEntry),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FullStateEntry {
    path: String,
    #[serde(rename = "type", default)]
    kind: StateKind,
    init: Option<String>,
}

impl From<StateEntryForm> for StateEntry {
    fn from(form: StateEntryForm) -> Self {
        match form {
            StateEntryForm::Path(path) => Self {
                path,
                kind: StateKind::Dir,
                init: None,
            },
            StateEntryForm::Full(entry) => Self {
                path: entry.path,
                kind: entry.kind,
                init: entry.init,
            },
        }
    }
}

/// A host path shown at the same path inside the container. Written as
/// `path`, `path:ro` or `path:rw`, as with docker's `-v`, or in full;
/// read-only unless `rw`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "MountEntryForm")]
pub struct MountEntry {
    /// `~/…` or absolute.
    pub path: String,
    pub mode: MountMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MountMode {
    #[default]
    Ro,
    Rw,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MountEntryForm {
    Path(String),
    Full(FullMountEntry),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FullMountEntry {
    path: String,
    #[serde(default)]
    mode: MountMode,
}

impl TryFrom<MountEntryForm> for MountEntry {
    type Error = String;

    fn try_from(form: MountEntryForm) -> Result<Self, Self::Error> {
        match form {
            MountEntryForm::Path(short) => Ok(match short.rsplit_once(':') {
                None => Self {
                    path: short,
                    mode: MountMode::Ro,
                },
                Some((path, "ro")) => Self {
                    path: path.to_owned(),
                    mode: MountMode::Ro,
                },
                Some((path, "rw")) => Self {
                    path: path.to_owned(),
                    mode: MountMode::Rw,
                },
                Some((_, suffix)) => {
                    return Err(format!(
                        "mount `{short}` ends in `:{suffix}`; the mode is `:ro` or `:rw`"
                    ));
                }
            }),
            MountEntryForm::Full(entry) => Ok(Self {
                path: entry.path,
                mode: entry.mode,
            }),
        }
    }
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
        let config: Self =
            serde_saphyr::from_str(text).context("invalid repository configuration")?;
        config.check_state()?;
        config.check_mounts()?;
        Ok(config)
    }

    /// Each state path is well formed, appears once, and has `init` only if
    /// it is a file.
    fn check_state(&self) -> anyhow::Result<()> {
        let mut seen = BTreeSet::new();
        for entry in &self.state {
            let path = &entry.path;
            check_path("state", path)?;
            ensure!(seen.insert(path), "state path `{path}` is listed twice");
            ensure!(
                entry.init.is_none() || entry.kind == StateKind::File,
                "state path `{path}` has `init`, which only a `type: file` takes"
            );
        }
        Ok(())
    }

    /// Each mount path is well formed and appears once.
    fn check_mounts(&self) -> anyhow::Result<()> {
        let mut seen = BTreeSet::new();
        for entry in &self.mounts {
            let path = &entry.path;
            check_path("mount", path)?;
            ensure!(seen.insert(path), "mount path `{path}` is listed twice");
        }
        Ok(())
    }
}

/// `~/…` or absolute, naming a place below it without `.`, `..` or `//`.
fn check_path(what: &str, path: &str) -> anyhow::Result<()> {
    let below = path
        .strip_prefix(HOME_PREFIX)
        .or_else(|| path.strip_prefix('/'))
        .with_context(|| format!("{what} path `{path}` must start with `~/` or `/`"))?;
    ensure!(
        below
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | "..")),
        "{what} path `{path}` must name a folder or file, without `.`, `..` or `//`"
    );
    Ok(())
}

/// A path written in `vz.yml` for the host: `~/…` under the home, absolute as
/// it is, anything else relative to `base`, the file's folder.
pub fn resolve_host_path(path: &str, base: &Path, home: &Path) -> PathBuf {
    if path.starts_with(HOME_PREFIX) || path.starts_with('/') {
        expand_path(path, home)
    } else {
        base.join(path)
    }
}

/// `~/x` under the home; an absolute path as it is. The home has the same
/// path inside the container as on the host.
pub fn expand_path(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix(HOME_PREFIX) {
        Some(below_home) => home.join(below_home),
        None => PathBuf::from(path),
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
    fn parse__state_short_and_full_forms__reads_path_type_and_init() {
        let text = "image: alpine\n\
                    state:\n  \
                    - ~/.claude\n  \
                    - /var/cache/apt\n  \
                    - { path: ~/.local/share/fish, type: dir }\n  \
                    - { path: ~/.claude.json, type: file, init: \"{}\" }\n";

        let config = RepoConfig::parse(text).unwrap();

        let entry = |path: &str, kind, init: Option<&str>| StateEntry {
            path: path.to_owned(),
            kind,
            init: init.map(str::to_owned),
        };
        let expected = vec![
            entry("~/.claude", StateKind::Dir, None),
            entry("/var/cache/apt", StateKind::Dir, None),
            entry("~/.local/share/fish", StateKind::Dir, None),
            entry("~/.claude.json", StateKind::File, Some("{}")),
        ];
        assert_eq!(config.state, expected);
    }

    #[test]
    fn parse__invalid_state_path__is_refused() {
        let paths = [
            "relative", "~", "~/", "/", "~/../x", "/a/./b", "~user/x", "/a//b", "~/x/",
        ];
        for path in paths {
            let text = format!("image: alpine\nstate:\n  - \"{path}\"\n");

            let result = RepoConfig::parse(&text);

            assert!(result.is_err(), "path: {path}");
        }
    }

    #[test]
    fn parse__state_path_twice__is_refused() {
        let text = "image: alpine\nstate:\n  - ~/.claude\n  - { path: ~/.claude }\n";

        let error = RepoConfig::parse(text).unwrap_err();

        assert!(error.to_string().contains("twice"), "{error}");
    }

    #[test]
    fn parse__init_on_a_folder__is_refused() {
        let text = "image: alpine\nstate:\n  - { path: ~/.claude, init: x }\n";

        let error = RepoConfig::parse(text).unwrap_err();

        assert!(error.to_string().contains("init"), "{error}");
    }

    #[test]
    fn parse__mount_mode_suffix__docker_semantics() {
        let text =
            "image: alpine\nmounts:\n  - ~/repos\n  - ~/.gitconfig:ro\n  - ~/.config/gh:rw\n";

        let config = RepoConfig::parse(text).unwrap();

        let entry = |path: &str, mode| MountEntry {
            path: path.to_owned(),
            mode,
        };
        let expected = vec![
            entry("~/repos", MountMode::Ro),
            entry("~/.gitconfig", MountMode::Ro),
            entry("~/.config/gh", MountMode::Rw),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn parse__mount_unknown_suffix__is_refused_naming_it() {
        let text = "image: alpine\nmounts:\n  - ~/.config/gh:wr\n";

        let error = RepoConfig::parse(text).unwrap_err();

        assert!(format!("{error:#}").contains(":wr"), "{error:#}");
    }

    #[test]
    fn parse__invalid_mount__is_refused() {
        let texts = [
            "mounts: [relative]",
            "mounts: [\"~/a/../b\"]",
            "mounts: [{ path: ~/a, mode: wr }]",
            "mounts: [~/a, { path: ~/a, mode: rw }]",
        ];
        for text in texts {
            let result = RepoConfig::parse(&format!("image: alpine\n{text}\n"));

            assert!(result.is_err(), "text: {text}");
        }
    }

    #[test]
    fn resolve_host_path__relative_home_and_absolute() {
        let cases = [
            (
                ".vz_state",
                "/home/sally/repos/app/examples/state/.vz_state",
            ),
            ("~/.cache/vz", "/home/sally/.cache/vz"),
            ("/var/cache/vz", "/var/cache/vz"),
        ];
        for (path, expected) in cases {
            let resolved = resolve_host_path(
                path,
                Path::new("/home/sally/repos/app/examples/state"),
                Path::new("/home/sally"),
            );

            assert_eq!(resolved, PathBuf::from(expected), "path: {path}");
        }
    }

    /// The recipes in `examples/` and the repository's own `vz.yml` stay valid
    /// as the schema changes.
    #[test]
    fn load__every_example_and_the_repository_file__is_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let examples = std::fs::read_dir(root.join("examples")).unwrap();
        let files = examples
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .map(|dir| dir.join("vz.yml"))
            .chain([root.join("vz.yml")]);
        for file in files {
            let result = RepoConfig::load(&file);

            assert!(result.is_ok(), "{}: {result:?}", file.display());
        }
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
