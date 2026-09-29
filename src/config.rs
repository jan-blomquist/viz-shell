//! `vz.yml`: a root layer and named profiles, each a [`Layer`] of the same
//! shape. [`RepoConfig::effective`] merges the root with a profile's chain
//! and resolves the result into the [`EffectiveConfig`] vz runs with.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::constants::{DEFAULT_BUILD_CONTEXT, DEFAULT_TAG, HOME_PREFIX};

/// The parsed `vz.yml`, checked: every path well formed, every `extends`
/// naming a profile, no cycles.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoConfig {
    root: Layer,
}

/// One layer of configuration: the root of `vz.yml`, or a profile. Every
/// field is optional; collections are maps keyed by what they are about, so
/// a later layer changes or removes an entry by naming it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageSource>,
    /// Where state is kept: relative to the file's folder, `~/…` or
    /// absolute. Unset: `.vz_state` at the git root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
    /// Keyed by container path.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub state: BTreeMap<String, StateValue>,
    /// Keyed by host path, shown at the same path inside.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mounts: BTreeMap<String, MountValue>,
    /// Profiles only: the profile this one starts from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// Root only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Layer>,
}

/// Where the image comes from: a reference to pull, or a Dockerfile to build.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ImageSource {
    Reference(String),
    Build(BuildSpec),
}

/// Paths are relative to the folder of the `vz.yml`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSpec {
    pub dockerfile: PathBuf,
    #[serde(default = "default_build_context")]
    pub context: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, String>,
}

fn default_build_context() -> PathBuf {
    PathBuf::from(DEFAULT_BUILD_CONTEXT)
}

/// A state entry as written: `true` or `false`, `dir` or `file`, or in full.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StateValue {
    Enabled(bool),
    Kind(StateKind),
    Full(StateSpec),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    #[serde(rename = "type", default)]
    pub kind: StateKind,
    /// A file's content when vz creates it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StateKind {
    #[default]
    Dir,
    File,
}

/// A mount as written: `true` (read-only) or `false`, `ro` or `rw`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MountValue {
    Enabled(bool),
    Mode(MountMode),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MountMode {
    #[default]
    Ro,
    Rw,
}

/// The configuration vz runs with: every layer applied, `false` entries gone,
/// shorthands spelled out.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveConfig {
    pub image: ImageSource,
    pub state_dir: Option<String>,
    pub state: Vec<StateEntry>,
    pub mounts: Vec<MountEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateEntry {
    /// `~/…` or absolute.
    pub path: String,
    pub kind: StateKind,
    pub init: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MountEntry {
    /// `~/…` or absolute.
    pub path: String,
    pub mode: MountMode,
}

impl RepoConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let root: Layer =
            serde_saphyr::from_str(text).context("invalid repository configuration")?;
        ensure!(
            root.extends.is_none(),
            "`extends` belongs in a profile; the root is what every profile starts from"
        );
        root.check_entries()?;
        for (name, profile) in &root.profiles {
            ensure!(
                profile.profiles.is_empty(),
                "profile `{name}` holds `profiles`; profiles do not nest"
            );
            profile
                .check_entries()
                .with_context(|| format!("in profile `{name}`"))?;
        }
        let config = Self { root };
        for name in config.root.profiles.keys() {
            config.chain(name)?;
        }
        Ok(config)
    }

    /// The root, then the profile's `extends` chain from its start, then the
    /// profile itself: later layers win per field and per key.
    pub fn effective(&self, profile: Option<&str>) -> anyhow::Result<EffectiveConfig> {
        let chain = match profile {
            Some(name) => self.chain(name)?,
            None => Vec::new(),
        };
        let merged = chain
            .into_iter()
            .fold(self.root.clone(), |base, over| base.merge(over));
        EffectiveConfig::resolve(merged)
    }

    /// The profile and those it extends, the first one extended first.
    fn chain(&self, name: &str) -> anyhow::Result<Vec<&Layer>> {
        let mut names: Vec<&str> = Vec::new();
        let mut chain = Vec::new();
        let mut next = Some(name);
        while let Some(name) = next {
            if names.contains(&name) {
                bail!(
                    "profiles extend each other in a cycle: {} → {name}",
                    names.join(" → ")
                );
            }
            let layer = self.profile(name)?;
            names.push(name);
            chain.push(layer);
            next = layer.extends.as_deref();
        }
        chain.reverse();
        Ok(chain)
    }

    fn profile(&self, name: &str) -> anyhow::Result<&Layer> {
        self.root.profiles.get(name).with_context(|| {
            let known: Vec<&str> = self.root.profiles.keys().map(String::as_str).collect();
            match known.as_slice() {
                [] => format!("no profile `{name}`: vz.yml defines none"),
                _ => format!("no profile `{name}`; vz.yml defines {}", known.join(", ")),
            }
        })
    }
}

impl Layer {
    /// `over` on top of `self`: its fields where set, its entries per key.
    /// The result is a plain layer: no `extends`, no profiles.
    fn merge(self, over: &Layer) -> Layer {
        let mut state = self.state;
        state.extend(over.state.clone());
        let mut mounts = self.mounts;
        mounts.extend(over.mounts.clone());
        Layer {
            image: over.image.clone().or(self.image),
            state_dir: over.state_dir.clone().or(self.state_dir),
            state,
            mounts,
            extends: None,
            profiles: BTreeMap::new(),
        }
    }

    /// Paths are well formed, and `init` is given only to a file.
    fn check_entries(&self) -> anyhow::Result<()> {
        for (path, value) in &self.state {
            check_path("state", path)?;
            if let StateValue::Full(spec) = value {
                ensure!(
                    spec.init.is_none() || spec.kind == StateKind::File,
                    "state path `{path}` has `init`, which only a `type: file` takes"
                );
            }
        }
        for path in self.mounts.keys() {
            check_path("mount", path)?;
        }
        Ok(())
    }
}

impl EffectiveConfig {
    fn resolve(layer: Layer) -> anyhow::Result<Self> {
        let image = layer
            .image
            .context("no image: set `image:` in vz.yml, or in the profile")?;
        let state = layer
            .state
            .into_iter()
            .filter_map(|(path, value)| {
                let (kind, init) = match value {
                    StateValue::Enabled(false) => return None,
                    StateValue::Enabled(true) => (StateKind::Dir, None),
                    StateValue::Kind(kind) => (kind, None),
                    StateValue::Full(spec) => (spec.kind, spec.init),
                };
                Some(StateEntry { path, kind, init })
            })
            .collect();
        let mounts = layer
            .mounts
            .into_iter()
            .filter_map(|(path, value)| {
                let mode = match value {
                    MountValue::Enabled(false) => return None,
                    MountValue::Enabled(true) => MountMode::Ro,
                    MountValue::Mode(mode) => mode,
                };
                Some(MountEntry { path, mode })
            })
            .collect();
        Ok(Self {
            image,
            state_dir: layer.state_dir,
            state,
            mounts,
        })
    }
}

impl EffectiveConfig {
    /// As a `vz.yml` without profiles: every shorthand spelled out, so it
    /// parses back to the same configuration.
    pub fn to_yaml(&self) -> anyhow::Result<String> {
        serde_saphyr::to_string(&self.to_layer()).context("writing the effective configuration")
    }

    fn to_layer(&self) -> Layer {
        let state = self
            .state
            .iter()
            .map(|entry| {
                let value = match &entry.init {
                    None => StateValue::Kind(entry.kind),
                    Some(init) => StateValue::Full(StateSpec {
                        kind: entry.kind,
                        init: Some(init.clone()),
                    }),
                };
                (entry.path.clone(), value)
            })
            .collect();
        let mounts = self
            .mounts
            .iter()
            .map(|entry| (entry.path.clone(), MountValue::Mode(entry.mode)))
            .collect();
        Layer {
            image: Some(self.image.clone()),
            state_dir: self.state_dir.clone(),
            state,
            mounts,
            ..Layer::default()
        }
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

    fn effective(text: &str, profile: Option<&str>) -> EffectiveConfig {
        RepoConfig::parse(text).unwrap().effective(profile).unwrap()
    }

    fn error(text: &str) -> String {
        format!("{:#}", RepoConfig::parse(text).unwrap_err())
    }

    fn state(path: &str, kind: StateKind, init: Option<&str>) -> StateEntry {
        StateEntry {
            path: path.to_owned(),
            kind,
            init: init.map(str::to_owned),
        }
    }

    fn mount(path: &str, mode: MountMode) -> MountEntry {
        MountEntry {
            path: path.to_owned(),
            mode,
        }
    }

    const BASE: &str = "\
image: debian
mounts:
  ~/repos: ro
  ~/.gitconfig: ro
state:
  ~/.cache: dir
profiles:
  writable:
    mounts:
      ~/repos: rw
  bare:
    mounts:
      ~/repos: false
      ~/.gitconfig: false
    state:
      ~/.cache: false
  bare-alpine:
    extends: bare
    image: alpine
    state:
      ~/scratch: dir
";

    #[test]
    fn effective__image_reference__reads_reference() {
        let config = effective("image: hello-world\n", None);

        assert_eq!(
            config.image,
            ImageSource::Reference("hello-world".to_owned())
        );
    }

    #[test]
    fn effective__dockerfile_only__defaults_context_and_args() {
        let config = effective("image:\n  dockerfile: Dockerfile\n", None);

        let expected = BuildSpec {
            dockerfile: PathBuf::from("Dockerfile"),
            context: PathBuf::from("."),
            args: BTreeMap::new(),
        };
        assert_eq!(config.image, ImageSource::Build(expected));
    }

    #[test]
    fn effective__state_value_forms__spelled_out_and_false_dropped() {
        let text = "\
image: debian
state:
  ~/.a: true
  ~/.b: dir
  ~/.c.json: file
  ~/.d.json: { type: file, init: \"{}\" }
  /opt/data: { type: dir }
  ~/.gone: false
";

        let config = effective(text, None);

        let expected = vec![
            state("/opt/data", StateKind::Dir, None),
            state("~/.a", StateKind::Dir, None),
            state("~/.b", StateKind::Dir, None),
            state("~/.c.json", StateKind::File, None),
            state("~/.d.json", StateKind::File, Some("{}")),
        ];
        assert_eq!(config.state, expected);
    }

    #[test]
    fn effective__mount_value_forms__true_is_read_only_and_false_dropped() {
        let text = "image: debian\nmounts:\n  ~/a: true\n  ~/b: ro\n  ~/c: rw\n  ~/d: false\n";

        let config = effective(text, None);

        let expected = vec![
            mount("~/a", MountMode::Ro),
            mount("~/b", MountMode::Ro),
            mount("~/c", MountMode::Rw),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn effective__no_profile__is_the_root() {
        let config = effective(BASE, None);

        let expected = vec![
            mount("~/.gitconfig", MountMode::Ro),
            mount("~/repos", MountMode::Ro),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn effective__profile_setting_a_key__overrides_the_root_entry() {
        let config = effective(BASE, Some("writable"));

        let expected = vec![
            mount("~/.gitconfig", MountMode::Ro),
            mount("~/repos", MountMode::Rw),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn effective__profile_setting_false__removes_the_root_entry() {
        let config = effective(BASE, Some("bare"));

        assert_eq!((config.mounts, config.state), (vec![], vec![]));
    }

    #[test]
    fn effective__extends_chain__applies_root_then_extended_then_profile() {
        let config = effective(BASE, Some("bare-alpine"));

        let expected = EffectiveConfig {
            image: ImageSource::Reference("alpine".to_owned()),
            state_dir: None,
            state: vec![state("~/scratch", StateKind::Dir, None)],
            mounts: vec![],
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn to_yaml__effective_configuration__parses_back_to_itself() {
        let config = effective(BASE, Some("writable"));

        let yaml = config.to_yaml().unwrap();

        assert_eq!(effective(&yaml, None), config, "{yaml}");
    }

    #[test]
    fn to_yaml__shorthands__are_spelled_out() {
        let text = "image: debian\nstate:\n  ~/.a: true\nmounts:\n  ~/b: true\n";

        let yaml = effective(text, None).to_yaml().unwrap();

        assert!(
            yaml.contains("~/.a: dir") && yaml.contains("~/b: ro"),
            "{yaml}"
        );
    }

    #[test]
    fn effective__unknown_profile__is_refused_naming_the_defined_ones() {
        let config = RepoConfig::parse(BASE).unwrap();

        let error = config.effective(Some("nope")).unwrap_err().to_string();

        assert!(
            error.contains("`nope`") && error.contains("bare, bare-alpine, writable"),
            "{error}"
        );
    }

    #[test]
    fn effective__no_image_anywhere__is_refused() {
        let config = RepoConfig::parse("mounts:\n  ~/repos: ro\n").unwrap();

        let error = config.effective(None).unwrap_err().to_string();

        assert!(error.contains("no image"), "{error}");
    }

    #[test]
    fn effective__image_only_in_a_profile__resolves_with_it() {
        let text = "profiles:\n  ci:\n    image: alpine\n";

        let config = effective(text, Some("ci"));

        assert_eq!(config.image, ImageSource::Reference("alpine".to_owned()));
    }

    #[test]
    fn parse__unknown_key__is_refused_naming_it() {
        let cases = [
            ("image: debian\nimgae: typo\n", "imgae"),
            ("profiles:\n  ci:\n    mounst: {}\n", "mounst"),
        ];
        for (text, key) in cases {
            assert!(error(text).contains(key), "key: {key}: {}", error(text));
        }
    }

    #[test]
    fn parse__unknown_key_in_build__is_refused() {
        let text = "image:\n  dockerfile: Dockerfile\n  dockerfle: typo\n";

        let result = RepoConfig::parse(text);

        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn parse__invalid_state_path__is_refused_naming_it() {
        let paths = [
            "relative", "~", "~/", "/", "~/../x", "/a/./b", "~user/x", "/a//b", "~/x/",
        ];
        for path in paths {
            let text = format!("image: debian\nstate:\n  \"{path}\": dir\n");

            let message = error(&text);

            assert!(
                message.contains(&format!("`{path}`")),
                "path: {path}: {message}"
            );
        }
    }

    #[test]
    fn parse__invalid_path_in_a_profile__is_refused_naming_the_profile() {
        let text = "image: debian\nprofiles:\n  ci:\n    mounts:\n      relative: ro\n";

        let message = error(text);

        assert!(
            message.contains("profile `ci`") && message.contains("`relative`"),
            "{message}"
        );
    }

    #[test]
    fn parse__init_on_a_folder__is_refused() {
        let text = "image: debian\nstate:\n  ~/.x: { init: x }\n";

        assert!(error(text).contains("init"), "{}", error(text));
    }

    #[test]
    fn parse__unknown_mount_mode__is_refused() {
        let text = "image: debian\nmounts:\n  ~/.config/gh: wr\n";

        let result = RepoConfig::parse(text);

        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn parse__misplaced_profile_keys__are_refused() {
        let cases = [
            (
                "extends: ci\nprofiles:\n  ci: {}\n",
                "`extends` belongs in a profile",
            ),
            ("profiles:\n  ci:\n    profiles: { x: {} }\n", "do not nest"),
            ("profiles:\n  ci:\n    extends: nope\n", "no profile `nope`"),
            (
                "profiles:\n  a: { extends: b }\n  b: { extends: a }\n",
                "cycle",
            ),
        ];
        for (text, expected) in cases {
            let message = error(text);

            assert!(
                message.contains(expected),
                "expected {expected:?}: {message}"
            );
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
    /// as the schema changes, with every profile they define.
    #[test]
    fn load__every_example_and_the_repository_file__resolves_with_each_profile() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let examples = std::fs::read_dir(root.join("examples")).unwrap();
        let files = examples
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .map(|dir| dir.join("vz.yml"))
            .chain([root.join("vz.yml")]);
        for file in files {
            let config = RepoConfig::load(&file).unwrap_or_else(|e| panic!("{e:#}"));
            let profiles = [None]
                .into_iter()
                .chain(config.root.profiles.keys().map(|name| Some(name.as_str())));
            for profile in profiles {
                let result = config.effective(profile);

                assert!(result.is_ok(), "{} {profile:?}: {result:?}", file.display());
            }
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
