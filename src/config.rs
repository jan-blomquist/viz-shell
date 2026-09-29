//! Configuration: the global file and the repository's, each a root [`Layer`]
//! with named profiles of the same shape. [`Config::effective`] applies them
//! in order and resolves the result into the [`EffectiveConfig`] vz runs with:
//! global root, repo root, then for each profile of the chosen one's
//! `extends` chain, its global section, then its repo section. A profile is a
//! mode: each file says what it adds in it.
//!
//! Collections are lists of entries, each keyed by its path or name: a bare
//! entry for the common case, the expanded form for anything else, and
//! `enabled: false` to remove one. A later layer's entry with the same key
//! updates the earlier one in its place; a new one comes last. Two maps:
//! `env.defaults`, keyed by variable name, and `profiles`, keyed by profile
//! name.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::constants::{DEFAULT_BUILD_CONTEXT, DEFAULT_TAG, HOME_PREFIX};

/// The configuration files in effect, each optional, checked: every
/// `extends` names a profile of either file, and none forms a cycle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    global: Option<Layer>,
    repo: Option<Layer>,
}

/// A profile as `vz profiles` shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileInfo {
    pub name: String,
    /// `global`, `repo`, or both, in that order.
    pub defined_in: Vec<&'static str>,
    pub extends: Option<String>,
    /// What its sections change, in a few words each: `sudo`, `2 mounts`, …
    pub changes: Vec<String>,
}

/// One layer of configuration: the root of `vz.yml`, or a profile. Every
/// field is optional.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// Profiles only: the profile this one starts from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageSource>,
    /// Where state is kept: relative to the file's folder, `~/…` or
    /// absolute. Unset: `.vz_state` at the git root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
    /// The viz-shell banner above an interactive shell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<bool>,
    /// The interactive shell: a name on the image's PATH, or an absolute
    /// path. Unset, or missing from the image: bash, else sh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// The container outlives the shell that created it; `vz kill` removes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistent: Option<bool>,
    /// A plain `vz` joins a container of this repository and profile, when
    /// one runs or is kept, instead of creating another.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attach: Option<bool>,
    /// What of the host the shell shares.
    #[serde(default, skip_serializing_if = "Share::is_unset")]
    pub share: Share,
    /// What the shell may do inside: nothing beyond the secure floor unless
    /// a layer grants it.
    #[serde(default, skip_serializing_if = "Privileges::is_unset")]
    pub privileges: Privileges,
    /// Environment variables inside the container.
    #[serde(default, skip_serializing_if = "Env::is_unset")]
    pub env: Env,
    /// Container paths whose contents outlive the container.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<StateItem>,
    /// Host paths shown at the same path inside.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<MountItem>,
    /// Root only, keyed by profile name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Layer>,
}

/// An entry of a list that later layers change by its key.
trait Keyed: Clone {
    /// The path or name that identifies the entry.
    fn key(&self) -> &str;
    fn key_mut(&mut self) -> &mut String;
    fn enabled(&self) -> bool;
}

/// `over`'s entries on top of `base`'s: one with a key already there updates
/// that entry in its place; a new one comes last.
fn merge_keyed<T: Keyed>(base: &[T], over: &[T]) -> Vec<T> {
    let mut merged = base.to_vec();
    for entry in over {
        match merged
            .iter_mut()
            .find(|earlier| earlier.key() == entry.key())
        {
            Some(earlier) => *earlier = entry.clone(),
            None => merged.push(entry.clone()),
        }
    }
    merged
}

/// Refuses a key listed twice in one list.
fn check_unique<T: Keyed>(list: &[T], what: &str) -> anyhow::Result<()> {
    for (index, entry) in list.iter().enumerate() {
        ensure!(
            !list[..index]
                .iter()
                .any(|earlier| earlier.key() == entry.key()),
            "{what} `{}` is listed twice",
            entry.key()
        );
    }
    Ok(())
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// What of the host the shell may share: a fixed set, so a misspelt key is
/// refused. Each is off unless a layer turns it on.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    /// The host's docker daemon: its socket, joined through its group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker: Option<bool>,
    /// The host's network stack, instead of docker's own network: the
    /// host's `localhost` and ports. Without it the shell still reaches the
    /// internet, through docker's network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_network: Option<bool>,
}

impl Share {
    fn is_unset(&self) -> bool {
        *self == Share::default()
    }

    fn merge(&self, over: &Share) -> Share {
        Share {
            docker: over.docker.or(self.docker),
            host_network: over.host_network.or(self.host_network),
        }
    }
}

/// What the shell may do inside the container: a fixed set, so a misspelt
/// key is refused. Each is off unless a layer grants it; off, the container
/// runs on the secure floor.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Privileges {
    /// Root through sudo: docker's default capabilities instead of none, and
    /// a sudoers line for the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sudo: Option<bool>,
}

impl Privileges {
    fn is_unset(&self) -> bool {
        self.sudo.is_none()
    }

    fn merge(&self, over: &Privileges) -> Privileges {
        Privileges {
            sudo: over.sudo.or(self.sudo),
        }
    }
}

/// Environment variables, from three sources; later wins: `defaults`, then
/// `files` in order, then `passthrough`. Values from files and the host are
/// read on the host, never mounted, and never printed.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Env {
    /// Values written here: the lowest level. Keyed by variable name;
    /// `null` removes one.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, Option<EnvScalar>>,
    /// `.env`-style files, read in order; optional unless `required: true`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileItem>,
    /// Host variables copied in, by name or `*`/`?` glob.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passthrough: Vec<PassthroughItem>,
}

impl Env {
    fn is_unset(&self) -> bool {
        *self == Env::default()
    }

    fn merge(&self, over: &Env) -> Env {
        let mut defaults = self.defaults.clone();
        defaults.extend(over.defaults.clone());
        Env {
            defaults,
            files: merge_keyed(&self.files, &over.files),
            passthrough: merge_keyed(&self.passthrough, &over.passthrough),
        }
    }
}

/// A value as YAML writes it; it reaches the container as its text.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EnvScalar {
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
}

impl EnvScalar {
    fn to_value(&self) -> String {
        match self {
            EnvScalar::Bool(value) => value.to_string(),
            EnvScalar::Integer(value) => value.to_string(),
            EnvScalar::Float(value) => value.to_string(),
            EnvScalar::Text(value) => value.clone(),
        }
    }
}

/// An env file: a bare path, optional, or expanded.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FileItem {
    Path(String),
    Full(FileSpec),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileSpec {
    pub path: String,
    /// Refuse to start when the file is missing, rather than skip it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

impl Keyed for FileItem {
    fn key(&self) -> &str {
        match self {
            FileItem::Path(path) => path,
            FileItem::Full(spec) => &spec.path,
        }
    }

    fn key_mut(&mut self) -> &mut String {
        match self {
            FileItem::Path(key) => key,
            FileItem::Full(spec) => &mut spec.path,
        }
    }

    fn enabled(&self) -> bool {
        !matches!(self, FileItem::Full(spec) if !spec.enabled)
    }
}

/// A passthrough: a bare name or glob, or expanded.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PassthroughItem {
    Name(String),
    Full(PassthroughSpec),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PassthroughSpec {
    pub name: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

impl Keyed for PassthroughItem {
    fn key(&self) -> &str {
        match self {
            PassthroughItem::Name(name) => name,
            PassthroughItem::Full(spec) => &spec.name,
        }
    }

    fn key_mut(&mut self) -> &mut String {
        match self {
            PassthroughItem::Name(key) => key,
            PassthroughItem::Full(spec) => &mut spec.name,
        }
    }

    fn enabled(&self) -> bool {
        !matches!(self, PassthroughItem::Full(spec) if !spec.enabled)
    }
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

/// A state entry: a bare path, a folder, or expanded.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StateItem {
    Path(String),
    Full(StateSpec),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    pub path: String,
    #[serde(rename = "type", default, skip_serializing_if = "StateKind::is_dir")]
    pub kind: StateKind,
    /// A file's content when vz creates it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init: Option<String>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StateKind {
    #[default]
    Dir,
    File,
}

impl StateKind {
    fn is_dir(&self) -> bool {
        *self == StateKind::Dir
    }
}

impl Keyed for StateItem {
    fn key(&self) -> &str {
        match self {
            StateItem::Path(path) => path,
            StateItem::Full(spec) => &spec.path,
        }
    }

    fn key_mut(&mut self) -> &mut String {
        match self {
            StateItem::Path(key) => key,
            StateItem::Full(spec) => &mut spec.path,
        }
    }

    fn enabled(&self) -> bool {
        !matches!(self, StateItem::Full(spec) if !spec.enabled)
    }
}

/// A mount: a bare path, read-only, or expanded. Keyed by where it lands
/// inside, so one host path can land in several places.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MountItem {
    Path(String),
    Full(MountSpec),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MountSpec {
    /// On the host.
    pub path: String,
    /// Inside the container; the same as `path` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "MountMode::is_ro")]
    pub mode: MountMode,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MountMode {
    #[default]
    Ro,
    Rw,
}

impl MountMode {
    fn is_ro(&self) -> bool {
        *self == MountMode::Ro
    }
}

impl Keyed for MountItem {
    /// The path inside.
    fn key(&self) -> &str {
        match self {
            MountItem::Path(path) => path,
            MountItem::Full(spec) => spec.target.as_deref().unwrap_or(&spec.path),
        }
    }

    fn key_mut(&mut self) -> &mut String {
        match self {
            MountItem::Path(key) => key,
            MountItem::Full(spec) => spec.target.as_mut().unwrap_or(&mut spec.path),
        }
    }

    fn enabled(&self) -> bool {
        !matches!(self, MountItem::Full(spec) if !spec.enabled)
    }
}

/// The configuration vz runs with: every layer applied, removed entries gone,
/// shorthands spelled out.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveConfig {
    /// The layers applied, in order: `global root`, `repo trusted`, …
    pub layers: Vec<String>,
    pub image: ImageSource,
    pub state_dir: Option<String>,
    pub banner: bool,
    pub shell: Option<String>,
    pub persistent: bool,
    pub attach: bool,
    pub share: Shared,
    pub privileges: Granted,
    pub env: EffectiveEnv,
    pub state: Vec<StateEntry>,
    pub mounts: Vec<MountEntry>,
}

/// What the shell shares with the host.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shared {
    pub docker: bool,
    pub host_network: bool,
}

/// What the shell may do inside.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Granted {
    pub sudo: bool,
}

/// The environment's sources, settled: no removed entries, values as text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectiveEnv {
    /// By name.
    pub defaults: Vec<(String, String)>,
    /// In order.
    pub files: Vec<EnvFile>,
    /// Names and globs, in order.
    pub passthrough: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvFile {
    /// As written: relative to the `vz.yml`'s folder, `~/…` or absolute.
    pub path: String,
    pub required: bool,
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
    /// On the host: `~/…` or absolute.
    pub path: String,
    /// Inside, when not the same path.
    pub target: Option<String>,
    pub mode: MountMode,
}

impl Config {
    /// Checks the profiles across both files.
    pub fn new(global: Option<Layer>, repo: Option<Layer>) -> anyhow::Result<Self> {
        let config = Self { global, repo };
        for name in config.profile_names() {
            config.chain(&name)?;
        }
        Ok(config)
    }

    /// Global root, repo root, then for each profile of the chosen one's
    /// chain, from its start: its global section, then its repo section.
    /// Later layers win per field and per key.
    pub fn effective(&self, profile: Option<&str>) -> anyhow::Result<EffectiveConfig> {
        let chain = match profile {
            Some(name) => self.chain(name)?,
            None => Vec::new(),
        };
        let mut layers: Vec<(String, &Layer)> = Vec::new();
        layers.extend(
            self.global
                .iter()
                .map(|root| ("global root".to_owned(), root)),
        );
        layers.extend(self.repo.iter().map(|root| ("repo root".to_owned(), root)));
        for name in &chain {
            for (origin, file) in self.files() {
                if let Some(section) = file.profiles.get(name) {
                    layers.push((format!("{origin} {name}"), section));
                }
            }
        }
        let merged = layers
            .iter()
            .fold(Layer::default(), |base, (_, over)| base.merge(over));
        let mut effective = EffectiveConfig::resolve(merged)?;
        effective.layers = layers.into_iter().map(|(label, _)| label).collect();
        Ok(effective)
    }

    /// Every profile, with the files that define it and what it extends.
    pub fn profiles(&self) -> Vec<ProfileInfo> {
        self.profile_names()
            .into_iter()
            .map(|name| ProfileInfo {
                defined_in: self
                    .files()
                    .filter(|(_, file)| file.profiles.contains_key(&name))
                    .map(|(origin, _)| origin)
                    .collect(),
                extends: self.extends(&name).map(str::to_owned),
                changes: self
                    .files()
                    .filter_map(|(_, file)| file.profiles.get(&name))
                    .fold(Layer::default(), |base, section| base.merge(section))
                    .changes(),
                name,
            })
            .collect()
    }

    fn files(&self) -> impl Iterator<Item = (&'static str, &Layer)> {
        [
            ("global", self.global.as_ref()),
            ("repo", self.repo.as_ref()),
        ]
        .into_iter()
        .filter_map(|(origin, file)| Some((origin, file?)))
    }

    fn profile_names(&self) -> BTreeSet<String> {
        self.files()
            .flat_map(|(_, file)| file.profiles.keys().cloned())
            .collect()
    }

    /// The repo's `extends` for a profile, else the global one's.
    fn extends(&self, name: &str) -> Option<&str> {
        self.files()
            .filter_map(|(_, file)| file.profiles.get(name)?.extends.as_deref())
            .last()
    }

    /// The profile and those it extends, the first one extended first.
    fn chain(&self, name: &str) -> anyhow::Result<Vec<String>> {
        let known = self.profile_names();
        let mut chain: Vec<String> = Vec::new();
        let mut next = Some(name);
        while let Some(name) = next {
            if chain.iter().any(|seen| seen == name) {
                bail!(
                    "profiles extend each other in a cycle: {} → {name}",
                    chain.join(" → ")
                );
            }
            if !known.contains(name) {
                let known: Vec<&str> = known.iter().map(String::as_str).collect();
                bail!(match known.as_slice() {
                    [] => format!("no profile `{name}`: no configuration defines one"),
                    _ => format!("no profile `{name}`; defined: {}", known.join(", ")),
                });
            }
            chain.push(name.to_owned());
            next = self.extends(name);
        }
        chain.reverse();
        Ok(chain)
    }
}

impl Layer {
    /// One configuration file, checked on its own: `extends` only in
    /// profiles, profiles not nested, paths and names well formed.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let root: Layer = serde_saphyr::from_str(text).context("invalid configuration")?;
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
        Ok(root)
    }

    /// Reads, checks, and makes every host path absolute: relative to the
    /// file's own folder, `~/…` under the home, `${repo}` and `${home}`
    /// substituted. Only then do entries of different files match by path.
    pub fn load(path: &Path, home: &Path, repo_root: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut layer = Self::parse(&text).with_context(|| format!("in {}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("/"));
        layer
            .resolve_paths(dir, home, repo_root)
            .with_context(|| format!("in {}", path.display()))?;
        Ok(layer)
    }

    fn resolve_paths(&mut self, dir: &Path, home: &Path, repo_root: &Path) -> anyhow::Result<()> {
        let host_path = |path: &str| -> anyhow::Result<String> {
            let path = substitute(path, repo_root, home)?;
            Ok(resolve_host_path(&path, dir, home)
                .to_string_lossy()
                .into_owned())
        };
        if let Some(ImageSource::Build(spec)) = &mut self.image {
            spec.dockerfile = dir.join(&spec.dockerfile);
            spec.context = dir.join(&spec.context);
        }
        if let Some(state_dir) = &self.state_dir {
            self.state_dir = Some(host_path(state_dir)?);
        }
        for entry in &mut self.env.files {
            let path = host_path(entry.key())?;
            *entry.key_mut() = path;
        }
        for entry in &mut self.state {
            let path = expand_path(entry.key(), home)
                .to_string_lossy()
                .into_owned();
            *entry.key_mut() = path;
        }
        let expand = |path: &str| expand_path(path, home).to_string_lossy().into_owned();
        for entry in &mut self.mounts {
            match entry {
                MountItem::Path(path) => *path = expand(path),
                MountItem::Full(spec) => {
                    spec.path = expand(&spec.path);
                    spec.target = spec.target.as_deref().map(expand);
                }
            }
        }
        for profile in self.profiles.values_mut() {
            profile.resolve_paths(dir, home, repo_root)?;
        }
        Ok(())
    }

    /// What this layer changes, in a few words each, for `vz profiles`.
    fn changes(&self) -> Vec<String> {
        let switch = |on: Option<bool>, name: &str| {
            on.map(|on| match on {
                true => name.to_owned(),
                false => format!("no {name}"),
            })
        };
        let count = |entries: usize, what: &str| match entries {
            0 => None,
            1 => Some(format!("1 {what}")),
            _ => Some(format!("{entries} {what}s")),
        };
        let image = self.image.as_ref().map(|image| match image {
            ImageSource::Reference(reference) => format!("image {reference}"),
            ImageSource::Build(_) => "its own image".to_owned(),
        });
        let shell = self.shell.as_ref().map(|shell| format!("shell {shell}"));
        [
            image,
            shell,
            switch(self.privileges.sudo, "sudo"),
            switch(self.share.docker, "docker"),
            switch(self.share.host_network, "host network"),
            count(self.mounts.len(), "mount"),
            count(self.state.len(), "state path"),
            count(self.env.defaults.len(), "env default"),
            count(self.env.files.len(), "env file"),
            count(self.env.passthrough.len(), "passthrough"),
            switch(self.banner, "banner"),
            switch(self.persistent, "persistent"),
            switch(self.attach, "attach"),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// `over` on top of `self`: its fields where set, its entries per key.
    /// The result is a plain layer: no `extends`, no profiles.
    fn merge(self, over: &Layer) -> Layer {
        Layer {
            extends: None,
            image: over.image.clone().or(self.image),
            state_dir: over.state_dir.clone().or(self.state_dir),
            banner: over.banner.or(self.banner),
            shell: over.shell.clone().or(self.shell),
            persistent: over.persistent.or(self.persistent),
            attach: over.attach.or(self.attach),
            share: self.share.merge(&over.share),
            privileges: self.privileges.merge(&over.privileges),
            env: self.env.merge(&over.env),
            state: merge_keyed(&self.state, &over.state),
            mounts: merge_keyed(&self.mounts, &over.mounts),
            profiles: BTreeMap::new(),
        }
    }

    /// Paths and names are well formed, each once per list, and `init` is
    /// given only to a file.
    fn check_entries(&self) -> anyhow::Result<()> {
        if let Some(shell) = &self.shell {
            ensure!(
                is_shell(shell),
                "shell `{shell}` is neither a name like `fish` nor an absolute path"
            );
        }
        check_unique(&self.state, "state path")?;
        for entry in &self.state {
            check_path("state", entry.key())?;
            if let StateItem::Full(spec) = entry {
                ensure!(
                    spec.init.is_none() || spec.kind == StateKind::File,
                    "state path `{}` has `init`, which only a `type: file` takes",
                    spec.path
                );
            }
        }
        check_unique(&self.mounts, "mount")?;
        for entry in &self.mounts {
            check_path("mount", entry.key())?;
            if let MountItem::Full(spec) = entry {
                check_path("mount", &spec.path)?;
            }
        }
        for name in self.env.defaults.keys() {
            ensure!(
                is_env_name(name),
                "env default `{name}` is not a variable name: letters, digits and `_`, \
                 not starting with a digit"
            );
        }
        check_unique(&self.env.passthrough, "env passthrough")?;
        for entry in &self.env.passthrough {
            let pattern = entry.key();
            ensure!(
                !pattern.is_empty()
                    && pattern
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '*' | '?')),
                "env passthrough `{pattern}` is not a variable name or a `*`/`?` glob"
            );
        }
        check_unique(&self.env.files, "env file")?;
        for entry in &self.env.files {
            ensure!(!entry.key().is_empty(), "env file path is empty");
        }
        Ok(())
    }
}

/// A program name, found on the image's PATH, or an absolute path.
fn is_shell(shell: &str) -> bool {
    !shell.is_empty()
        && !shell.contains(char::is_whitespace)
        && (shell.starts_with('/') || !shell.contains('/'))
}

/// A path under the home as `~/…`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(below) => format!("~/{}", below.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Letters, digits and `_`, not starting with a digit.
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl EffectiveConfig {
    fn resolve(layer: Layer) -> anyhow::Result<Self> {
        let image = layer
            .image
            .context("no image: set `image:` in the global or the repository configuration")?;
        let state = layer
            .state
            .iter()
            .filter(|entry| entry.enabled())
            .map(|entry| match entry {
                StateItem::Path(path) => StateEntry {
                    path: path.clone(),
                    kind: StateKind::Dir,
                    init: None,
                },
                StateItem::Full(spec) => StateEntry {
                    path: spec.path.clone(),
                    kind: spec.kind,
                    init: spec.init.clone(),
                },
            })
            .collect();
        let mounts = layer
            .mounts
            .iter()
            .filter(|entry| entry.enabled())
            .map(|entry| match entry {
                MountItem::Path(path) => MountEntry {
                    path: path.clone(),
                    target: None,
                    mode: MountMode::Ro,
                },
                MountItem::Full(spec) => MountEntry {
                    path: spec.path.clone(),
                    target: spec.target.clone().filter(|target| *target != spec.path),
                    mode: spec.mode,
                },
            })
            .collect();
        Ok(Self {
            layers: Vec::new(),
            image,
            state_dir: layer.state_dir,
            banner: layer.banner.unwrap_or(false),
            shell: layer.shell,
            persistent: layer.persistent.unwrap_or(false),
            attach: layer.attach.unwrap_or(false),
            share: Shared {
                docker: layer.share.docker.unwrap_or(false),
                host_network: layer.share.host_network.unwrap_or(false),
            },
            privileges: Granted {
                sudo: layer.privileges.sudo.unwrap_or(false),
            },
            env: EffectiveEnv::resolve(layer.env),
            state,
            mounts,
        })
    }
}

impl EffectiveEnv {
    fn resolve(env: Env) -> Self {
        let defaults = env
            .defaults
            .into_iter()
            .filter_map(|(name, value)| Some((name, value?.to_value())))
            .collect();
        let files = env
            .files
            .iter()
            .filter(|entry| entry.enabled())
            .map(|entry| EnvFile {
                path: entry.key().to_owned(),
                required: matches!(entry, FileItem::Full(spec) if spec.required),
            })
            .collect();
        let passthrough = env
            .passthrough
            .iter()
            .filter(|entry| entry.enabled())
            .map(|entry| entry.key().to_owned())
            .collect();
        Self {
            defaults,
            files,
            passthrough,
        }
    }

    fn to_env(&self) -> Env {
        Env {
            defaults: self
                .defaults
                .iter()
                .map(|(name, value)| (name.clone(), Some(EnvScalar::Text(value.clone()))))
                .collect(),
            files: self
                .files
                .iter()
                .map(|file| match file.required {
                    true => FileItem::Full(FileSpec {
                        path: file.path.clone(),
                        required: true,
                        enabled: true,
                    }),
                    false => FileItem::Path(file.path.clone()),
                })
                .collect(),
            passthrough: self
                .passthrough
                .iter()
                .map(|name| PassthroughItem::Name(name.clone()))
                .collect(),
        }
    }
}

impl EffectiveConfig {
    /// As a `vz.yml` without profiles: every entry in its shortest form that
    /// says the same, so it parses back to the same configuration.
    pub fn to_yaml(&self) -> anyhow::Result<String> {
        serde_saphyr::to_string(&self.to_layer()).context("writing the effective configuration")
    }

    fn to_layer(&self) -> Layer {
        let state = self
            .state
            .iter()
            .map(|entry| match (entry.kind, &entry.init) {
                (StateKind::Dir, None) => StateItem::Path(entry.path.clone()),
                (kind, init) => StateItem::Full(StateSpec {
                    path: entry.path.clone(),
                    kind,
                    init: init.clone(),
                    enabled: true,
                }),
            })
            .collect();
        let mounts = self
            .mounts
            .iter()
            .map(|entry| match (&entry.target, entry.mode) {
                (None, MountMode::Ro) => MountItem::Path(entry.path.clone()),
                (target, mode) => MountItem::Full(MountSpec {
                    path: entry.path.clone(),
                    target: target.clone(),
                    mode,
                    enabled: true,
                }),
            })
            .collect();
        Layer {
            image: Some(self.image.clone()),
            state_dir: self.state_dir.clone(),
            banner: self.banner.then_some(true),
            shell: self.shell.clone(),
            persistent: self.persistent.then_some(true),
            attach: self.attach.then_some(true),
            share: Share {
                docker: self.share.docker.then_some(true),
                host_network: self.share.host_network.then_some(true),
            },
            privileges: Privileges {
                sudo: self.privileges.sudo.then_some(true),
            },
            env: self.env.to_env(),
            state,
            mounts,
            ..Layer::default()
        }
    }
}

/// The global configuration a first run writes.
pub const DEFAULT_GLOBAL: &str = r#"# The global configuration: what every repository starts from. viz-shell wrote
# it on its first run and never overwrites it: edit it freely. A repository's own configuration comes on
# top: its root over this root, its profiles over these of the same name.
# Relative paths here are relative to this folder.

# The default mode: plain `vz`, untrusted. Nothing of the host but the repo.
image: debian:stable-slim

# A banner above an interactive shell, like fastfetch: the repository, branch,
# profile, image, shell, and what the shell shares and may do. Never above
# `vz -- command`.
banner: true

# The interactive shell for every repository: a name on the image's PATH, or an
# absolute path. An image without it gives a warning, then bash, else sh.
# shell: fish

env:
  files:
    # Variables for every repository; skipped while the file does not exist.
    - environment
  passthrough:
    - EDITOR

# A mount must exist on the host, or vz refuses to start: uncomment what you have.
# mounts:
#   - ~/.gitconfig          # your git identity, read-only

profiles:
  # `vz --profile trusted`: for repositories you trust. A repository extends it
  # with its own `trusted:`, or with `extends: trusted` in another profile.
  trusted:
    privileges:
      # Root through sudo, with docker's default capabilities, instead of the
      # secure floor, where the shell holds none. The image needs sudo.
      sudo: true
    share:
      # The host's docker daemon: root-equivalent control of the host.
      docker: true
      # The host's network: its localhost and ports. Without it the shell still
      # reaches the internet, through docker's network.
      host_network: true
    mounts:
      # ssh as on the host: keys readable inside.
      - ~/.ssh
    env:
      files:
        # Secrets only trusted mode sees; skipped while the file does not exist.
        - trusted.env
      passthrough:
        - GH_TOKEN
"#;

/// Writes the default global configuration when there is none; never
/// overwrites. Returns whether it wrote.
pub fn scaffold_global(path: &Path) -> anyhow::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, DEFAULT_GLOBAL).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// `${repo}` and `${home}`; any other `${…}` is an error. A `$` not followed
/// by `{` stays as it is.
pub fn substitute(text: &str, repo_root: &Path, home: &Path) -> anyhow::Result<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .with_context(|| format!("`{text}` has a `${{` without `}}`"))?;
        match &after[..end] {
            "repo" => out.push_str(&repo_root.to_string_lossy()),
            "home" => out.push_str(&home.to_string_lossy()),
            other => bail!("`${{{other}}}` in `{text}`: vz substitutes ${{repo}} and ${{home}}"),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
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

    /// A repository configuration alone.
    fn repo(text: &str) -> Config {
        Config::new(None, Some(Layer::parse(text).unwrap())).unwrap()
    }

    fn effective(text: &str, profile: Option<&str>) -> EffectiveConfig {
        repo(text).effective(profile).unwrap()
    }

    fn error(text: &str) -> String {
        let result = Layer::parse(text).and_then(|layer| Config::new(None, Some(layer)));
        format!("{:#}", result.unwrap_err())
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
            target: None,
            mode,
        }
    }

    const BASE: &str = "\
image: debian
mounts:
  - ~/repos
  - ~/.gitconfig
state:
  - ~/.cache
profiles:
  writable:
    mounts:
      - { path: ~/repos, mode: rw }
  bare:
    mounts:
      - { path: ~/repos, enabled: false }
      - { path: ~/.gitconfig, enabled: false }
    state:
      - { path: ~/.cache, enabled: false }
  bare-alpine:
    extends: bare
    image: alpine
    state:
      - ~/scratch
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
    fn effective__state_forms__spelled_out_in_order_disabled_dropped() {
        let text = "\
image: debian
state:
  - ~/.b
  - ~/.a
  - { path: ~/.c.json, type: file }
  - { path: ~/.d.json, type: file, init: \"{}\" }
  - { path: /opt/data, type: dir }
  - { path: ~/.gone, enabled: false }
";

        let config = effective(text, None);

        let expected = vec![
            state("~/.b", StateKind::Dir, None),
            state("~/.a", StateKind::Dir, None),
            state("~/.c.json", StateKind::File, None),
            state("~/.d.json", StateKind::File, Some("{}")),
            state("/opt/data", StateKind::Dir, None),
        ];
        assert_eq!(config.state, expected);
    }

    #[test]
    fn effective__mount_forms__bare_is_read_only_disabled_dropped() {
        let text = "\
image: debian
mounts:
  - ~/a
  - { path: ~/b, mode: ro }
  - { path: ~/c, mode: rw }
  - { path: ~/d, enabled: false }
";

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
            mount("~/repos", MountMode::Ro),
            mount("~/.gitconfig", MountMode::Ro),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn effective__profile_entry_with_the_same_path__updates_it_in_place() {
        let config = effective(BASE, Some("writable"));

        let expected = vec![
            mount("~/repos", MountMode::Rw),
            mount("~/.gitconfig", MountMode::Ro),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn effective__profile_entry_disabled__removes_the_root_entry() {
        let config = effective(BASE, Some("bare"));

        assert_eq!((config.mounts, config.state), (vec![], vec![]));
    }

    #[test]
    fn effective__extends_chain__applies_root_then_extended_then_profile() {
        let config = effective(BASE, Some("bare-alpine"));

        let expected = EffectiveConfig {
            layers: vec![
                "repo root".to_owned(),
                "repo bare".to_owned(),
                "repo bare-alpine".to_owned(),
            ],
            image: ImageSource::Reference("alpine".to_owned()),
            state_dir: None,
            banner: false,
            shell: None,
            persistent: false,
            attach: false,
            share: Shared::default(),
            privileges: Granted::default(),
            env: EffectiveEnv::default(),
            state: vec![state("~/scratch", StateKind::Dir, None)],
            mounts: vec![],
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn effective__share_docker__off_unless_a_layer_turns_it_on() {
        let text = "\
image: debian
share:
  docker: true
profiles:
  offline:
    share: { docker: false }
  again:
    extends: offline
    share: { docker: true }
";
        let cases = [
            ("image: debian\n", None, false),
            (text, None, true),
            (text, Some("offline"), false),
            (text, Some("again"), true),
        ];
        for (text, profile, expected) in cases {
            assert_eq!(
                effective(text, profile).share.docker,
                expected,
                "profile: {profile:?}"
            );
        }
    }

    #[test]
    fn effective__sudo__off_unless_a_profile_grants_it() {
        let text = "\
image: debian
profiles:
  trusted:
    privileges: { sudo: true }
  locked:
    extends: trusted
    privileges: { sudo: false }
";
        let cases = [
            (None, false),
            (Some("trusted"), true),
            (Some("locked"), false),
        ];
        for (profile, expected) in cases {
            assert_eq!(
                effective(text, profile).privileges.sudo,
                expected,
                "profile: {profile:?}"
            );
        }
    }

    #[test]
    fn parse__unknown_privilege__is_refused_naming_it() {
        let text = "image: debian\nprivileges:\n  root: true\n";

        assert!(error(text).contains("root"), "{}", error(text));
    }

    #[test]
    fn effective__shell__the_last_layer_to_set_it() {
        let text = "\
image: debian
shell: fish
profiles:
  plain:
    shell: /bin/sh
  inherits: {}
";
        let cases = [
            (None, Some("fish")),
            (Some("plain"), Some("/bin/sh")),
            (Some("inherits"), Some("fish")),
        ];
        for (profile, expected) in cases {
            assert_eq!(
                effective(text, profile).shell.as_deref(),
                expected,
                "profile: {profile:?}"
            );
        }
        assert_eq!(effective("image: debian\n", None).shell, None);
    }

    #[test]
    fn parse__shell_neither_a_name_nor_absolute__is_refused() {
        for shell in ["bin/fish", "\"\"", "\"fish -l\""] {
            let text = format!("image: debian\nshell: {shell}\n");

            let message = error(&text);

            assert!(message.contains("absolute path"), "{shell}: {message}");
        }
    }

    #[test]
    fn effective__persistent_and_attach__off_unless_a_layer_turns_them_on() {
        let text = "\
image: debian
profiles:
  kept:
    persistent: true
  shared:
    extends: kept
    attach: true
";
        let cases = [
            (None, (false, false)),
            (Some("kept"), (true, false)),
            (Some("shared"), (true, true)),
        ];
        for (profile, expected) in cases {
            let config = effective(text, profile);

            assert_eq!(
                (config.persistent, config.attach),
                expected,
                "profile: {profile:?}"
            );
        }
    }

    #[test]
    fn effective__banner__off_unless_a_layer_turns_it_on() {
        let text = "\
image: debian
banner: true
profiles:
  quiet:
    banner: false
";
        let cases = [(None, true), (Some("quiet"), false)];
        for (profile, expected) in cases {
            assert_eq!(
                effective(text, profile).banner,
                expected,
                "profile: {profile:?}"
            );
        }
        assert!(!effective("image: debian\n", None).banner);
    }

    #[test]
    fn effective__share_host_network__off_unless_a_layer_turns_it_on() {
        let text = "\
image: debian
profiles:
  trusted:
    share: { host_network: true }
";
        let cases = [(None, false), (Some("trusted"), true)];
        for (profile, expected) in cases {
            assert_eq!(
                effective(text, profile).share.host_network,
                expected,
                "profile: {profile:?}"
            );
        }
    }

    #[test]
    fn parse__unknown_share__is_refused_naming_it() {
        let text = "image: debian\nshare:\n  dcoker: true\n";

        assert!(error(text).contains("dcoker"), "{}", error(text));
    }

    const ENV: &str = "\
image: debian
env:
  defaults:
    RUST_LOG: info
    PORT: 8080
    DEBUG: false
    GONE: x
  files:
    - .env
    - ~/.secrets/a.env
    - { path: .env.old, required: true }
  passthrough:
    - GH_TOKEN
    - \"FMP_*\"
profiles:
  ci:
    env:
      defaults:
        RUST_LOG: debug
        GONE: null
      files:
        - { path: .env, required: true }
        - { path: .env.old, enabled: false }
        - .env.ci
      passthrough:
        - { name: GH_TOKEN, enabled: false }
        - CI_*
";

    fn defaults(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn file(path: &str, required: bool) -> EnvFile {
        EnvFile {
            path: path.to_owned(),
            required,
        }
    }

    #[test]
    fn effective__env_root__values_as_text_files_in_order() {
        let env = effective(ENV, None).env;

        let expected = EffectiveEnv {
            defaults: defaults(&[
                ("DEBUG", "false"),
                ("GONE", "x"),
                ("PORT", "8080"),
                ("RUST_LOG", "info"),
            ]),
            files: vec![
                file(".env", false),
                file("~/.secrets/a.env", false),
                file(".env.old", true),
            ],
            passthrough: vec!["GH_TOKEN".to_owned(), "FMP_*".to_owned()],
        };
        assert_eq!(env, expected);
    }

    #[test]
    fn effective__env_profile__overrides_removes_and_appends() {
        let env = effective(ENV, Some("ci")).env;

        let expected = EffectiveEnv {
            // RUST_LOG overridden, GONE removed by null.
            defaults: defaults(&[("DEBUG", "false"), ("PORT", "8080"), ("RUST_LOG", "debug")]),
            // .env made required in its place, .env.old removed, .env.ci appended.
            files: vec![
                file(".env", true),
                file("~/.secrets/a.env", false),
                file(".env.ci", false),
            ],
            // GH_TOKEN removed, CI_* appended.
            passthrough: vec!["FMP_*".to_owned(), "CI_*".to_owned()],
        };
        assert_eq!(env, expected);
    }

    #[test]
    fn parse__invalid_env_names__are_refused_naming_them() {
        let cases = [
            ("env:\n  defaults:\n    1ST: x\n", "`1ST`"),
            ("env:\n  passthrough: [MY-VAR]\n", "`MY-VAR`"),
            ("env:\n  set:\n    A: x\n", "set"),
        ];
        for (text, expected) in cases {
            let message = error(&format!("image: debian\n{text}"));

            assert!(message.contains(expected), "expected {expected}: {message}");
        }
    }

    #[test]
    fn parse__same_key_twice_in_one_list__is_refused_naming_it() {
        let cases = [
            ("mounts: [~/a, { path: ~/a, mode: rw }]", "mount `~/a`"),
            ("state: [~/a, ~/a]", "state path `~/a`"),
            ("env:\n  files: [.env, .env]", "env file `.env`"),
            ("env:\n  passthrough: [A, A]", "env passthrough `A`"),
        ];
        for (text, expected) in cases {
            let message = error(&format!("image: debian\n{text}\n"));

            assert!(
                message.contains(expected) && message.contains("twice"),
                "expected {expected}: {message}"
            );
        }
    }

    #[test]
    fn effective__unknown_profile__is_refused_naming_the_defined_ones() {
        let config = repo(BASE);

        let error = config.effective(Some("nope")).unwrap_err().to_string();

        assert!(
            error.contains("`nope`") && error.contains("bare, bare-alpine, writable"),
            "{error}"
        );
    }

    #[test]
    fn effective__no_image_anywhere__is_refused() {
        let config = repo("mounts: [~/repos]\n");

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
    fn to_yaml__effective_configuration__parses_back_to_itself() {
        let cases = [
            (format!("{BASE}share:\n  docker: true\n"), Some("writable")),
            (ENV.to_owned(), Some("ci")),
        ];
        for (text, profile) in cases {
            let config = effective(&text, profile);

            let yaml = config.to_yaml().unwrap();

            let reread = EffectiveConfig {
                layers: config.layers.clone(),
                ..effective(&yaml, None)
            };
            assert_eq!(reread, config, "{yaml}");
        }
    }

    const TARGETS: &str = "\
image: debian
mounts:
  - { path: ~/skills, target: ~/.agents/skills }
  - { path: ~/skills, target: ~/.claude/skills }
  - { path: ~/same, target: ~/same }
profiles:
  agents-only:
    mounts:
      - { path: ~/skills, target: ~/.claude/skills, enabled: false }
";

    #[test]
    fn effective__mount_targets__one_source_at_several_keyed_by_target() {
        let with_target = |target: &str| MountEntry {
            target: Some(target.to_owned()),
            ..mount("~/skills", MountMode::Ro)
        };
        let cases = [
            (
                None,
                vec![
                    with_target("~/.agents/skills"),
                    with_target("~/.claude/skills"),
                    mount("~/same", MountMode::Ro),
                ],
            ),
            (
                Some("agents-only"),
                vec![
                    with_target("~/.agents/skills"),
                    mount("~/same", MountMode::Ro),
                ],
            ),
        ];
        for (profile, expected) in cases {
            assert_eq!(effective(TARGETS, profile).mounts, expected, "{profile:?}");
        }
    }

    #[test]
    fn to_yaml__mount_targets__parse_back_to_themselves() {
        let config = effective(TARGETS, None);

        let yaml = config.to_yaml().unwrap();

        assert_eq!(effective(&yaml, None).mounts, config.mounts, "{yaml}");
    }

    #[test]
    fn parse__mount_target_or_source_not_absolute__is_refused() {
        for entry in [
            "{ path: ~/a, target: relative }",
            "{ path: relative, target: ~/a }",
        ] {
            let text = format!("image: debian\nmounts:\n  - {entry}\n");

            assert!(
                error(&text).contains("must start with"),
                "{entry}: {}",
                error(&text)
            );
        }
    }

    #[test]
    fn to_yaml__entries__in_their_shortest_form() {
        let text = "\
image: debian
state:
  - { path: ~/.a, type: dir }
mounts:
  - { path: ~/b, mode: ro }
  - { path: ~/c, mode: rw }
";

        let yaml = effective(text, None).to_yaml().unwrap();

        assert!(
            yaml.contains("- ~/.a") && yaml.contains("- ~/b") && yaml.contains("mode: rw"),
            "{yaml}"
        );
    }

    #[test]
    fn parse__unknown_key__is_refused_naming_it() {
        let cases = [
            ("image: debian\nimgae: typo\n", "imgae"),
            ("profiles:\n  ci:\n    mounst: []\n", "mounst"),
        ];
        for (text, key) in cases {
            assert!(error(text).contains(key), "key: {key}: {}", error(text));
        }
    }

    #[test]
    fn parse__unknown_key_in_build__is_refused() {
        let text = "image:\n  dockerfile: Dockerfile\n  dockerfle: typo\n";

        let result = Layer::parse(text);

        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn parse__invalid_state_path__is_refused_naming_it() {
        let paths = [
            "relative", "~", "~/", "/", "~/../x", "/a/./b", "~user/x", "/a//b", "~/x/",
        ];
        for path in paths {
            let text = format!("image: debian\nstate:\n  - \"{path}\"\n");

            let message = error(&text);

            assert!(
                message.contains(&format!("`{path}`")),
                "path: {path}: {message}"
            );
        }
    }

    #[test]
    fn parse__invalid_path_in_a_profile__is_refused_naming_the_profile() {
        let text = "image: debian\nprofiles:\n  ci:\n    mounts: [relative]\n";

        let message = error(text);

        assert!(
            message.contains("profile `ci`") && message.contains("`relative`"),
            "{message}"
        );
    }

    #[test]
    fn parse__init_on_a_folder__is_refused() {
        let text = "image: debian\nstate:\n  - { path: ~/.x, init: x }\n";

        assert!(error(text).contains("init"), "{}", error(text));
    }

    #[test]
    fn parse__unknown_mount_mode__is_refused() {
        let text = "image: debian\nmounts:\n  - { path: ~/.config/gh, mode: wr }\n";

        let result = Layer::parse(text);

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
    fn substitute__cases() {
        let cases = [
            ("plain", "plain"),
            ("${repo}/data", "/home/sally/repos/app/data"),
            ("${home}/.cache", "/home/sally/.cache"),
            ("cost: $5 and $HOME", "cost: $5 and $HOME"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                substitute(
                    text,
                    Path::new("/home/sally/repos/app"),
                    Path::new("/home/sally")
                )
                .unwrap(),
                expected,
                "text: {text}"
            );
        }
    }

    #[test]
    fn substitute__unknown_or_unclosed__is_refused_naming_it() {
        for (text, expected) in [
            ("${profile}", "${profile}"),
            ("${env:X}", "${env:X}"),
            ("${repo", "without"),
        ] {
            let error = substitute(
                text,
                Path::new("/home/sally/repos/app"),
                Path::new("/home/sally"),
            )
            .unwrap_err()
            .to_string();

            assert!(error.contains(expected), "{text}: {error}");
        }
    }

    #[test]
    fn scaffold_global__missing__writes_the_template() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".config/viz-shell/global.yml");

        let wrote = scaffold_global(&path).unwrap();

        assert!(wrote);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_GLOBAL);
    }

    #[test]
    fn scaffold_global__present__kept_as_it_is() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("global.yml");
        std::fs::write(&path, "image: mine\n").unwrap();

        let wrote = scaffold_global(&path).unwrap();

        assert!(!wrote);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "image: mine\n");
    }

    #[test]
    fn default_global__is_valid_with_each_profile() {
        let config = Config::new(Some(Layer::parse(DEFAULT_GLOBAL).unwrap()), None).unwrap();

        for profile in [None, Some("trusted")] {
            assert!(config.effective(profile).is_ok(), "profile: {profile:?}");
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

    /// The recipes in `examples/`, the repository's own configuration and the
    /// global configuration a first run writes stay valid as the schema
    /// changes, with every profile. A folder with neither kind of file is
    /// skipped.
    #[test]
    fn load__every_example_and_the_repository_file__resolves_with_each_profile() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let home = Path::new("/home/sally");
        let default_global = tempfile::tempdir().unwrap();
        std::fs::write(default_global.path().join("global.yml"), DEFAULT_GLOBAL).unwrap();
        let examples = std::fs::read_dir(root.join("examples")).unwrap();
        let dirs: Vec<PathBuf> = examples
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .chain([root.to_owned(), default_global.path().to_owned()])
            .collect();
        for dir in dirs {
            let read = |file: Option<PathBuf>| {
                file.map(|file| Layer::load(&file, home, root).unwrap_or_else(|e| panic!("{e:#}")))
            };
            // The repository file by the same precedence vz uses.
            let global_file = Some(dir.join("global.yml")).filter(|file| file.is_file());
            let (global, repo) = (read(global_file), read(crate::repo::config_file(&dir)));
            if global.is_none() && repo.is_none() {
                continue;
            }
            let config =
                Config::new(global, repo).unwrap_or_else(|e| panic!("{}: {e:#}", dir.display()));
            let profiles = [None].into_iter().chain(
                config
                    .profiles()
                    .into_iter()
                    .map(|profile| Some(profile.name)),
            );
            for profile in profiles {
                let result = config.effective(profile.as_deref());

                assert!(result.is_ok(), "{} {profile:?}: {result:?}", dir.display());
            }
        }
    }

    const GLOBAL: &str = "\
image: debian
env:
  defaults: { WHO: global, GLOBAL_ONLY: \"yes\" }
mounts:
  - ~/.gitconfig
profiles:
  trusted:
    share: { docker: true }
    env:
      files: [secrets.env]
  work:
    env:
      defaults: { WORK: \"yes\" }
";

    const REPO: &str = "\
env:
  defaults: { WHO: repo }
share:
  docker: false
mounts:
  - { path: ~/.gitconfig, enabled: false }
profiles:
  trusted:
    env:
      defaults: { WHO: repo-trusted }
  ci:
    extends: trusted
";

    const SALLY: &str = "/home/sally";
    const APP: &str = "/home/sally/repos/app";

    /// Each text read as `load` reads its file: paths resolved against its
    /// own folder.
    fn both(global: &str, repo: &str) -> Config {
        let read = |text: &str, dir: &str| {
            let mut layer = Layer::parse(text).unwrap();
            layer
                .resolve_paths(Path::new(dir), Path::new(SALLY), Path::new(APP))
                .unwrap();
            layer
        };
        let global = read(global, "/home/sally/.config/viz-shell");
        Config::new(Some(global), Some(read(repo, APP))).unwrap()
    }

    fn who(config: &EffectiveConfig) -> &str {
        let (_, value) = config
            .env
            .defaults
            .iter()
            .find(|(name, _)| name == "WHO")
            .unwrap();
        value
    }

    #[test]
    fn effective__no_profile__global_root_then_repo_root() {
        let config = both(GLOBAL, REPO).effective(None).unwrap();

        assert_eq!(config.layers, ["global root", "repo root"]);
        assert_eq!(config.image, ImageSource::Reference("debian".to_owned()));
        assert_eq!(who(&config), "repo");
    }

    #[test]
    fn effective__profile_in_both_files__global_section_then_repo_section() {
        let config = both(GLOBAL, REPO).effective(Some("trusted")).unwrap();

        assert_eq!(
            config.layers,
            ["global root", "repo root", "global trusted", "repo trusted"]
        );
        assert_eq!(who(&config), "repo-trusted");
    }

    #[test]
    fn effective__chosen_profile__beats_both_roots() {
        let config = both(GLOBAL, REPO).effective(Some("trusted")).unwrap();

        // The repo root turns docker off; the global trusted profile turns it on.
        assert!(config.share.docker);
    }

    #[test]
    fn effective__profile_only_in_the_global_file__applies_in_any_repo() {
        let config = both(GLOBAL, REPO).effective(Some("work")).unwrap();

        let work = config.env.defaults.iter().any(|(name, _)| name == "WORK");
        assert!(work, "{:?}", config.env.defaults);
    }

    #[test]
    fn effective__repo_profile_extending_a_global_one__applies_both_first() {
        let config = both(GLOBAL, REPO).effective(Some("ci")).unwrap();

        assert_eq!(
            config.layers,
            [
                "global root",
                "repo root",
                "global trusted",
                "repo trusted",
                "repo ci"
            ]
        );
        assert!(config.share.docker);
    }

    #[test]
    fn effective__relative_paths__resolved_against_their_own_file() {
        let config = both(GLOBAL, "env:\n  files: [.env]\n")
            .effective(Some("trusted"))
            .unwrap();

        let files: Vec<&str> = config
            .env
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(
            files,
            [
                "/home/sally/repos/app/.env",
                "/home/sally/.config/viz-shell/secrets.env"
            ]
        );
    }

    #[test]
    fn effective__repo_entry__disables_a_global_one_written_either_way() {
        let config = both(GLOBAL, REPO).effective(None).unwrap();

        // Global `~/.gitconfig`; the repo disables `~/.gitconfig` too: both
        // are /home/sally/.gitconfig once read.
        assert_eq!(config.mounts, vec![]);
    }

    #[test]
    fn profiles__both_files__each_with_where_it_is_defined() {
        let profiles = both(GLOBAL, REPO).profiles();

        let expected = vec![
            ProfileInfo {
                name: "ci".to_owned(),
                defined_in: vec!["repo"],
                extends: Some("trusted".to_owned()),
                changes: vec![],
            },
            ProfileInfo {
                name: "trusted".to_owned(),
                defined_in: vec!["global", "repo"],
                extends: None,
                // The global section's docker and file, the repo section's default.
                changes: vec![
                    "docker".to_owned(),
                    "1 env default".to_owned(),
                    "1 env file".to_owned(),
                ],
            },
            ProfileInfo {
                name: "work".to_owned(),
                defined_in: vec!["global"],
                extends: None,
                changes: vec!["1 env default".to_owned()],
            },
        ];
        assert_eq!(profiles, expected);
    }

    #[test]
    fn new__extends_a_profile_neither_file_defines__is_refused_naming_both_files_profiles() {
        let repo = Layer::parse("profiles:\n  ci:\n    extends: nope\n").unwrap();
        let global = Layer::parse(GLOBAL).unwrap();

        let error = Config::new(Some(global), Some(repo))
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("no profile `nope`; defined: ci, trusted, work"),
            "{error}"
        );
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
