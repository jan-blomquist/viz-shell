//! Step 2, parse: one file's YAML documents, each a configuration's
//! [`Layer`], checked on its own. Invariant: every key is known
//! (`deny_unknown_fields`), every entry well formed, `extends` names one
//! configuration, and the former `profiles:` block is refused.
//!
//! Collections are lists of entries, each keyed by its path or name: a bare
//! entry for the common case, the expanded form for anything else, and
//! `enabled: false` to remove one. One map: `env.defaults`, keyed by
//! variable name.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Deserializer, Serialize};

use super::merge::Keyed;
use crate::constants::{DEFAULT_BUILD_CONTEXT, DEFAULT_MOUNT_MODE, DEFAULT_TAG, HOME_PREFIX};

/// One configuration: a document of a file. Every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// What it is called; one configuration of a scope may go without, and
    /// is `default`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// In a library file only: more library folders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scan: Vec<String>,
    /// The configuration this one starts from.
    #[serde(
        default,
        deserialize_with = "one_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageSource>,
    /// Where state is kept: relative to the file's folder, `~/…` or
    /// absolute. Unset: `.vz_state` at the git root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
    /// The banner above an interactive shell: `true` for the built-in art,
    /// `false` for none, or art of its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<Banner>,
    /// The interactive shell: a name on the image's PATH, or an absolute
    /// path. Unset, or missing from the image: bash, else sh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// The container outlives the shell that created it; `vz kill` removes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistent: Option<bool>,
    /// `vz new` without a name joins a container of this repository and
    /// configuration, when one runs or is kept, instead of creating another.
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
    /// Commands run inside, as the user, in the repository.
    #[serde(default, skip_serializing_if = "Hooks::is_unset")]
    pub hooks: Hooks,
    /// The former `profiles:` block: read only to be refused by name.
    #[serde(default, skip_serializing)]
    pub profiles: Option<serde::de::IgnoredAny>,
}

/// `banner:` `true` for the built-in art, `false` for none, or art of its
/// own.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Banner {
    Switch(bool),
    Art(String),
}

/// `extends: trusted`; a list is refused.
fn one_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        One(String),
        #[allow(dead_code)] // only its shape matters: a list
        Many(Vec<serde::de::IgnoredAny>),
    }
    match Written::deserialize(deserializer)? {
        Written::One(name) => Ok(Some(name)),
        Written::Many(_) => Err(serde::de::Error::custom("extends takes one name")),
    }
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
    pub fn to_value(&self) -> String {
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

/// Commands run through `sh -c`, each list in order, keyed by the command.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// Once per container, on its first entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub create: Vec<HookItem>,
    /// Before every entry: each shell, and `vz -- command`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attach: Vec<HookItem>,
}

impl Hooks {
    fn is_unset(&self) -> bool {
        *self == Hooks::default()
    }
}

/// A hook: a bare command, or expanded.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum HookItem {
    Command(String),
    Full(HookSpec),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookSpec {
    pub run: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

/// Where the image comes from: a reference to pull, or a Dockerfile to build.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ImageSource {
    Reference(String),
    Build(BuildSpec),
}

/// Paths are relative to the folder of the file that sets them.
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

/// A mount: `path[:target][:ro|rw]`, [`DEFAULT_MOUNT_MODE`] unless it names
/// a mode, or expanded. Keyed by where it lands inside, so one host path can
/// land in several places. A string is parsed as it is read: `Path` is a bare
/// path in the default mode; anything else is `Full`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MountItem {
    Path(String),
    Full(MountSpec),
}

impl<'de> Deserialize<'de> for MountItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Written {
            Short(String),
            Full(MountSpec),
        }
        match Written::deserialize(deserializer)? {
            Written::Short(spec) => parse_mount(&spec)
                .map(MountItem::from)
                .map_err(serde::de::Error::custom),
            Written::Full(spec) => Ok(MountItem::Full(spec)),
        }
    }
}

impl From<MountSpec> for MountItem {
    /// The shortest form that says the same.
    fn from(spec: MountSpec) -> Self {
        match spec {
            MountSpec {
                path,
                target: None,
                mode,
                enabled: true,
            } if mode.is_default() => MountItem::Path(path),
            spec => MountItem::Full(spec),
        }
    }
}

/// Docker's `-v` grammar: `path[:target][:ro|rw]`. Each path starts with `/`
/// or `~`; [`check_path`] checks them further.
fn parse_mount(spec: &str) -> anyhow::Result<MountSpec> {
    let is_mode = |s: &str| matches!(s, "ro" | "rw");
    let is_path = |s: &str| s.starts_with('/') || s.starts_with('~');
    let fields: Vec<&str> = spec.split(':').collect();
    let (path, target, mode) = match fields.as_slice() {
        [path] if is_path(path) => (path, None, None),
        [path, mode] if is_path(path) && is_mode(mode) => (path, None, Some(mode)),
        [path, target] if is_path(path) && is_path(target) => (path, Some(target), None),
        [path, target, mode] if is_path(path) && is_path(target) && is_mode(mode) => {
            (path, Some(target), Some(mode))
        }
        _ => bail!("mount `{spec}`: expected path[:target][:ro|rw]"),
    };
    Ok(MountSpec {
        path: path.to_string(),
        target: target.map(|target| target.to_string()),
        mode: match mode {
            Some(&"ro") => MountMode::Ro,
            Some(_) => MountMode::Rw,
            None => MountMode::default(),
        },
        enabled: true,
    })
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MountSpec {
    /// On the host.
    pub path: String,
    /// Inside the container; the same as `path` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "MountMode::is_default")]
    pub mode: MountMode,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MountMode {
    Ro,
    Rw,
}

impl Default for MountMode {
    fn default() -> Self {
        DEFAULT_MOUNT_MODE
    }
}

impl MountMode {
    fn is_default(&self) -> bool {
        *self == MountMode::default()
    }
}

/// A file's documents, each a configuration, checked on its own. An empty
/// or comment-only document is none.
pub fn parse(text: &str) -> anyhow::Result<Vec<Layer>> {
    let documents: Vec<Layer> =
        serde_saphyr::from_multiple(text).context("invalid configuration")?;
    for (index, document) in documents.iter().enumerate() {
        let checked = check_document(document);
        match documents.len() {
            1 => checked?,
            _ => checked.with_context(|| format!("in document {}", index + 1))?,
        }
    }
    Ok(documents)
}

/// One document: its name well formed, no `profiles:`, entries well formed.
fn check_document(document: &Layer) -> anyhow::Result<()> {
    if let Some(name) = &document.name {
        ensure!(
            !name.is_empty() && !name.contains(char::is_whitespace),
            "name `{name}` is empty or holds whitespace"
        );
    }
    ensure!(
        document.profiles.is_none(),
        "`profiles:` is no more: write another document in the file, with `name:` and `extends:`"
    );
    document.check_entries()
}

impl Layer {
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
        for (kind, hooks) in [
            ("create", &self.hooks.create),
            ("attach", &self.hooks.attach),
        ] {
            check_unique(hooks, &format!("{kind} hook"))?;
            for entry in hooks {
                ensure!(!entry.key().trim().is_empty(), "{kind} hook is empty");
            }
        }
        Ok(())
    }
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

/// A program name, found on the image's PATH, or an absolute path.
fn is_shell(shell: &str) -> bool {
    !shell.is_empty()
        && !shell.contains(char::is_whitespace)
        && (shell.starts_with('/') || !shell.contains('/'))
}

/// Letters, digits and `_`, not starting with a digit.
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
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
    use crate::config::testing::message;

    /// Why `text` is refused.
    fn refusal(text: &str) -> String {
        message(parse(text))
    }

    /// Each document's name, `-` for none.
    fn names(text: &str) -> Vec<String> {
        parse(text)
            .unwrap()
            .into_iter()
            .map(|layer| layer.name.unwrap_or_else(|| "-".to_owned()))
            .collect()
    }

    #[test]
    fn parse__documents__one_configuration_each_empty_ones_none() {
        let cases: [(&str, &str, &[&str]); 7] = [
            ("one named", "name: a\n", &["a"]),
            ("one empty map", "{}\n", &["-"]),
            ("two named", "name: a\n---\nname: b\n", &["a", "b"]),
            ("a leading separator", "---\nname: a\n", &["a"]),
            (
                "a nameless one, then a named one",
                "image: debian\n---\nname: b\nextends: default\n",
                &["-", "b"],
            ),
            (
                "a comment-only document between two",
                "name: a\n---\n# only a comment\n---\nname: c\n",
                &["a", "c"],
            ),
            ("only a comment", "# only a comment\n", &[]),
        ];
        for (case, text, expected) in cases {
            let names = names(text);

            assert_eq!(names, expected, "{case}");
        }
    }

    #[test]
    fn parse__profiles_key__refused_saying_what_instead() {
        let message = refusal("image: debian\nprofiles:\n  ci: {}\n");

        assert!(
            message.contains(
                "`profiles:` is no more: write another document in the file, with `name:` and `extends:`"
            ),
            "{message}"
        );
    }

    #[test]
    fn parse__banner__a_switch_or_art_of_its_own() {
        let cases = [
            ("banner: true\n", Some(Banner::Switch(true))),
            ("banner: false\n", Some(Banner::Switch(false))),
            (
                "banner: \"== app ==\"\n",
                Some(Banner::Art("== app ==".to_owned())),
            ),
            ("banner: \"\"\n", Some(Banner::Art(String::new()))),
            (
                "banner: |\n  a\n  b\n",
                Some(Banner::Art("a\nb\n".to_owned())),
            ),
            ("{}\n", None),
        ];
        for (text, expected) in cases {
            let layer = parse(text).unwrap().remove(0);

            assert_eq!(layer.banner, expected, "{text:?}");
        }
    }

    #[test]
    fn parse__banner_neither_a_switch_nor_text__refused() {
        for text in ["banner: 1\n", "banner: [a]\n", "banner: { a: b }\n"] {
            let message = refusal(text);

            assert!(
                message.contains("untagged enum Banner"),
                "{text:?}: {message}"
            );
        }
    }

    #[test]
    fn parse__extends__one_name() {
        let cases = [("extends: trusted\n", Some("trusted")), ("{}\n", None)];
        for (text, expected) in cases {
            let layer = parse(text).unwrap().remove(0);

            assert_eq!(layer.extends.as_deref(), expected, "{text}");
        }
    }

    #[test]
    fn parse__extends_more_than_one_name__refused() {
        for text in ["extends: [default, trusted]\n", "extends: [default]\n"] {
            let message = refusal(text);

            assert!(
                message.contains("extends takes one name"),
                "{text}: {message}"
            );
        }
    }

    #[test]
    fn parse__name_empty_or_with_whitespace__refused() {
        for text in ["name: \"\"\n", "name: \"a b\"\n"] {
            let message = refusal(text);

            assert!(
                message.contains("is empty or holds whitespace"),
                "{text}: {message}"
            );
        }
    }

    #[test]
    fn parse__refusal_in_a_later_document__names_the_document() {
        let message = refusal("name: a\n---\nname: b\nmounts: [\"~/../x\"]\n");

        assert!(
            message.contains("in document 2: mount path `~/../x`"),
            "{message}"
        );
    }

    #[test]
    fn parse__unknown_privilege__is_refused_naming_it() {
        let message = refusal("image: debian\nprivileges:\n  root: true\n");

        assert!(message.contains("unknown field `root`"), "{message}");
    }

    #[test]
    fn parse__unknown_share__is_refused_naming_it() {
        let message = refusal("image: debian\nshare:\n  dcoker: true\n");

        assert!(message.contains("unknown field `dcoker`"), "{message}");
    }

    #[test]
    fn parse__shell_neither_a_name_nor_absolute__is_refused() {
        for shell in ["bin/fish", "\"\"", "\"fish -l\""] {
            let message = refusal(&format!("image: debian\nshell: {shell}\n"));

            assert!(
                message.contains("is neither a name like `fish` nor an absolute path"),
                "{shell}: {message}"
            );
        }
    }

    #[test]
    fn parse__invalid_env_names__are_refused_naming_them() {
        let cases = [
            ("env:\n  defaults:\n    1ST: x\n", "`1ST`"),
            ("env:\n  passthrough: [MY-VAR]\n", "`MY-VAR`"),
            ("env:\n  set:\n    A: x\n", "unknown field `set`"),
        ];
        for (text, expected) in cases {
            let message = refusal(&format!("image: debian\n{text}"));

            assert!(message.contains(expected), "expected {expected}: {message}");
        }
    }

    #[test]
    fn parse__same_key_twice_in_one_list__is_refused_naming_it() {
        let cases = [
            (
                "mounts: [~/a, { path: ~/a, mode: rw }]",
                "mount `~/a` is listed twice",
            ),
            ("state: [~/a, ~/a]", "state path `~/a` is listed twice"),
            (
                "env:\n  files: [.env, .env]",
                "env file `.env` is listed twice",
            ),
            (
                "env:\n  passthrough: [A, A]",
                "env passthrough `A` is listed twice",
            ),
            (
                "hooks:\n  attach: [ls, { run: ls }]",
                "attach hook `ls` is listed twice",
            ),
            ("mounts: [~/a, ~/b, ~/a]", "mount `~/a` is listed twice"),
        ];
        for (text, expected) in cases {
            let message = refusal(&format!("image: debian\n{text}\n"));

            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn parse__empty_hook__is_refused_naming_its_kind() {
        let cases = [
            ("hooks:\n  create: [\"\"]", "create hook is empty"),
            (
                "hooks:\n  attach: [{ run: \"  \" }]",
                "attach hook is empty",
            ),
            (
                "---\nname: ci\nhooks:\n  attach: [\" \"]",
                "attach hook is empty",
            ),
        ];
        for (text, expected) in cases {
            let message = refusal(&format!("image: debian\n{text}\n"));

            assert!(message.contains(expected), "expected {expected}: {message}");
        }
    }

    #[test]
    fn parse__unknown_key__is_refused_naming_it() {
        let cases = [
            ("image: debian\nimgae: typo\n", "unknown field `imgae`"),
            (
                "name: a\n---\nname: ci\nmounst: []\n",
                "unknown field `mounst`",
            ),
        ];
        for (text, expected) in cases {
            let message = refusal(text);

            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn parse__unknown_key_in_build__is_refused() {
        let message = refusal("image:\n  dockerfile: Dockerfile\n  dockerfle: typo\n");

        assert!(message.contains("untagged enum ImageSource"), "{message}");
    }

    #[test]
    fn parse__invalid_state_path__is_refused_naming_it() {
        let cases = [
            (
                "relative",
                "state path `relative` must start with `~/` or `/`",
            ),
            (
                "~user/x",
                "state path `~user/x` must start with `~/` or `/`",
            ),
            ("~", "state path `~` must start with `~/` or `/`"),
            ("~/", "state path `~/` must name a folder or file"),
            ("/", "state path `/` must name a folder or file"),
            ("~/../x", "state path `~/../x` must name a folder or file"),
            ("/a/./b", "state path `/a/./b` must name a folder or file"),
            ("/a//b", "state path `/a//b` must name a folder or file"),
            ("~/x/", "state path `~/x/` must name a folder or file"),
        ];
        for (path, expected) in cases {
            let message = refusal(&format!("image: debian\nstate:\n  - \"{path}\"\n"));

            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn parse__init_on_a_folder__is_refused() {
        let message = refusal("image: debian\nstate:\n  - { path: ~/.x, init: x }\n");

        assert!(
            message.contains("state path `~/.x` has `init`, which only a `type: file` takes"),
            "{message}"
        );
    }

    #[test]
    fn parse__unknown_mount_mode__is_refused() {
        let message = refusal("image: debian\nmounts:\n  - { path: ~/.config/gh, mode: wr }\n");

        // The refusal points at the entry: line 3, where the map starts.
        assert!(message.contains("line 3 column 5"), "{message}");
    }

    #[test]
    fn parse__mount_target_or_source_not_absolute__is_refused() {
        let cases = [
            (
                "{ path: ~/a, target: relative }",
                "mount path `relative` must start with `~/` or `/`",
            ),
            (
                "{ path: relative, target: ~/a }",
                "mount path `relative` must start with `~/` or `/`",
            ),
        ];
        for (entry, expected) in cases {
            let message = refusal(&format!("image: debian\nmounts:\n  - {entry}\n"));

            assert!(message.contains(expected), "{entry}: {message}");
        }
    }

    fn spec(path: &str, target: Option<&str>, mode: MountMode) -> MountSpec {
        MountSpec {
            path: path.to_owned(),
            target: target.map(str::to_owned),
            mode,
            enabled: true,
        }
    }

    #[test]
    fn parse_mount__every_accepted_form__yields_path_target_mode() {
        let cases = [
            ("~/repos", spec("~/repos", None, DEFAULT_MOUNT_MODE)),
            ("~/repos:ro", spec("~/repos", None, MountMode::Ro)),
            ("~/repos:rw", spec("~/repos", None, MountMode::Rw)),
            (
                "~/skills:~/.agents/skills",
                spec("~/skills", Some("~/.agents/skills"), DEFAULT_MOUNT_MODE),
            ),
            (
                "~/skills:~/.agents/skills:ro",
                spec("~/skills", Some("~/.agents/skills"), MountMode::Ro),
            ),
            (
                "~/skills:~/.config/opencode/skills:rw",
                spec("~/skills", Some("~/.config/opencode/skills"), MountMode::Rw),
            ),
            ("/srv/data", spec("/srv/data", None, DEFAULT_MOUNT_MODE)),
            (
                "/srv/data:/mnt/data:ro",
                spec("/srv/data", Some("/mnt/data"), MountMode::Ro),
            ),
        ];
        for (input, expected) in cases {
            let spec = parse_mount(input).unwrap();

            assert_eq!(spec, expected, "{input}");
        }
    }

    #[test]
    fn parse_mount__malformed_specs__are_refused_quoting_them() {
        let cases = [
            ("~/a:rx", "mount `~/a:rx`: expected path[:target][:ro|rw]"),
            (
                "~/a:~/b:~/c",
                "mount `~/a:~/b:~/c`: expected path[:target][:ro|rw]",
            ),
            ("~/a::rw", "mount `~/a::rw`: expected path[:target][:ro|rw]"),
            ("repos", "mount `repos`: expected path[:target][:ro|rw]"),
            (
                "~/a:repos",
                "mount `~/a:repos`: expected path[:target][:ro|rw]",
            ),
            (
                "~/a:ro:rw",
                "mount `~/a:ro:rw`: expected path[:target][:ro|rw]",
            ),
            ("", "mount ``: expected path[:target][:ro|rw]"),
            (":ro", "mount `:ro`: expected path[:target][:ro|rw]"),
        ];
        for (input, expected) in cases {
            let message = message(parse_mount(input));

            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn with_default_tag__no_tag_or_digest__gets_latest() {
        let cases = [
            ("hello-world", "hello-world:latest"),
            ("localhost:5000/base", "localhost:5000/base:latest"),
        ];
        for (reference, expected) in cases {
            let tagged = with_default_tag(reference);

            assert_eq!(tagged, expected, "reference: {reference}");
        }
    }

    #[test]
    fn with_default_tag__a_tag_or_digest__kept_as_it_is() {
        let references = [
            "hello-world:linux",
            "ghcr.io/org/base:1",
            "alpine@sha256:abc",
        ];
        for reference in references {
            let tagged = with_default_tag(reference);

            assert_eq!(tagged, reference);
        }
    }
}
