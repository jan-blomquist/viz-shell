//! A repository's containers: their names, the labels that identify them,
//! and which one a command means. Labels are the store: vz finds containers
//! by label, never by parsing names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::CUT;
use crate::constants::{
    CHAIN_LABEL, CONFIG_HASH_LABEL, CONFIG_LABEL, CONTAINER_PREFIX, DEFAULT_CONFIG, IMAGE_LABEL,
    INDEX_LABEL, MAX_LABEL_LEN, NAME_LABEL, PERSISTENT_LABEL, REPO_LABEL,
};

/// A hostname's limit; the name is the hostname too.
const MAX_NAME_LEN: usize = 63;

/// The longest name `vz new` takes.
const MAX_SESSION_NAME_LEN: usize = 24;

/// Hex digits of the configuration hash kept in a label.
const CONFIG_HASH_LEN: usize = 12;

/// One container of a repository, from its labels and `docker inspect`.
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub name: String,
    pub repo: PathBuf,
    pub index: u32,
    /// Given by `vz new NAME`.
    pub session_name: Option<String>,
    pub config: Option<String>,
    pub persistent: bool,
    /// The configuration's hash when it was created.
    pub config_hash: String,
    /// The configuration chain it was created with: `vz.chain`'s entries.
    pub chain: Vec<String>,
    /// Its image chain, tags bottom first: `vz.image`'s entries.
    pub images: Vec<String>,
    pub image: String,
    pub running: bool,
    /// When, in UTC: `2026-09-29 18:02`.
    pub created: String,
}

/// The part of `docker container inspect` vz reads.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Inspected {
    name: String,
    created: String,
    state: InspectedState,
    config: InspectedConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectedState {
    running: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectedConfig {
    image: String,
    labels: Option<HashMap<String, String>>,
}

/// vz's containers in `docker container inspect` output; one without an
/// index label is not one of vz's.
pub fn parse_inspect(json: &str) -> anyhow::Result<Vec<Container>> {
    let inspected: Vec<Inspected> =
        serde_json::from_str(json).context("reading docker inspect's output")?;
    Ok(inspected
        .into_iter()
        .filter_map(Container::from_inspected)
        .collect())
}

impl Container {
    fn from_inspected(inspected: Inspected) -> Option<Self> {
        let labels = inspected.config.labels.unwrap_or_default();
        let label = |name: &str| labels.get(name).filter(|value| !value.is_empty()).cloned();
        Some(Self {
            name: inspected.name.trim_start_matches('/').to_owned(),
            repo: PathBuf::from(label(REPO_LABEL)?),
            index: label(INDEX_LABEL)?.parse().ok()?,
            session_name: label(NAME_LABEL),
            config: label(CONFIG_LABEL),
            persistent: label(PERSISTENT_LABEL).as_deref() == Some("true"),
            config_hash: label(CONFIG_HASH_LABEL).unwrap_or_default(),
            chain: entries(label(CHAIN_LABEL)),
            images: entries(label(IMAGE_LABEL)),
            image: inspected.config.image,
            running: inspected.state.running,
            // RFC 3339, to the minute.
            created: inspected
                .created
                .get(..16)
                .unwrap_or_default()
                .replace('T', " "),
        })
    }
}

/// What a new container's labels say.
pub struct Labels<'a> {
    pub repo: &'a Path,
    pub index: u32,
    pub session_name: Option<&'a str>,
    pub config: Option<&'a str>,
    pub persistent: bool,
    /// The configuration's hash.
    pub config_hash: &'a str,
    /// The configuration chain's entries, `<config>@<file>`, in fold order.
    pub chain: &'a [String],
    /// The image chain's tags, bottom first.
    pub images: &'a [String],
}

impl Labels<'_> {
    /// Each label with its value, even when empty.
    pub fn pairs(&self) -> Vec<(String, String)> {
        [
            (REPO_LABEL, self.repo.display().to_string()),
            (INDEX_LABEL, self.index.to_string()),
            (NAME_LABEL, self.session_name.unwrap_or_default().to_owned()),
            (CONFIG_LABEL, self.config.unwrap_or_default().to_owned()),
            (PERSISTENT_LABEL, self.persistent.to_string()),
            (CONFIG_HASH_LABEL, self.config_hash.to_owned()),
            (CHAIN_LABEL, joined(self.chain)),
            (IMAGE_LABEL, joined(self.images)),
        ]
        .into_iter()
        .map(|(label, value)| (label.to_owned(), value))
        .collect()
    }
}

/// Entries joined by `,`, at most [`MAX_LABEL_LEN`] bytes. A longer list
/// loses entries from its middle, replaced by one `…`: the first and last
/// stay, which say where the chain starts and what it ends in.
fn joined(entries: &[String]) -> String {
    (0..=entries.len())
        .map(|cut| match cut {
            0 => entries.join(","),
            _ => {
                let kept = entries.len() - cut;
                let head = &entries[..kept.div_ceil(2)];
                let tail = &entries[entries.len() - kept / 2..];
                [head, &[CUT.to_owned()], tail].concat().join(",")
            }
        })
        .find(|label| label.len() <= MAX_LABEL_LEN)
        .unwrap_or_else(|| CUT.to_owned())
}

/// A label's entries; none when it is unset.
fn entries(label: Option<String>) -> Vec<String> {
    label
        .map(|label| label.split(',').map(str::to_owned).collect())
        .unwrap_or_default()
}

/// `vz-<index>-<repo>`, or `vz-<index>-<repo>-<name>`: the container's name
/// and hostname. The repository part is shortened to fit a hostname.
pub fn container_name(index: u32, repo_dir: &str, session_name: Option<&str>) -> String {
    let prefix = format!("{CONTAINER_PREFIX}{index}-");
    let suffix = session_name
        .map(|name| format!("-{name}"))
        .unwrap_or_default();
    let room = MAX_NAME_LEN - prefix.len() - suffix.len();
    let repo: String = slug(repo_dir).chars().take(room).collect();
    format!("{prefix}{}{suffix}", repo.trim_end_matches('-'))
}

/// Lowercase letters, digits and single dashes: what a hostname takes.
fn slug(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'a'..='z' | '0'..='9' => slug.push(c),
            _ if !slug.is_empty() && !slug.ends_with('-') => slug.push('-'),
            _ => {}
        }
    }
    let slug = slug.trim_end_matches('-');
    match slug.is_empty() {
        true => "repo".to_owned(),
        false => slug.to_owned(),
    }
}

/// A name for `vz new`: lowercase letters, digits and dashes, starting with a
/// letter, so it never reads as an index.
pub fn check_session_name(name: &str) -> anyhow::Result<()> {
    let valid = name.len() <= MAX_SESSION_NAME_LEN
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-');
    ensure!(
        valid,
        "`{name}` is not a container name: lowercase letters, digits and dashes, starting with \
         a letter, at most {MAX_SESSION_NAME_LEN} characters"
    );
    Ok(())
}

/// The lowest index no container of the repository has.
pub fn next_index(containers: &[Container], taken: &[u32]) -> u32 {
    (0..)
        .find(|index| {
            !taken.contains(index) && !containers.iter().any(|container| container.index == *index)
        })
        .unwrap_or_default()
}

/// A short hash of the effective configuration, to tell when a container
/// was created from another one.
pub fn config_hash(effective_yaml: &str) -> String {
    let mut hex = hex::encode(Sha256::digest(effective_yaml.as_bytes()));
    hex.truncate(CONFIG_HASH_LEN);
    hex
}

/// What `vz attach` or `vz kill` names: an index, or a name.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Index(u32),
    Name(String),
}

impl Target {
    pub fn parse(text: &str) -> Self {
        match text.parse() {
            Ok(index) => Self::Index(index),
            Err(_) => Self::Name(text.to_owned()),
        }
    }
}

/// The container `target` names: by index, by the name `vz new` gave it, or
/// by its full name.
pub fn find<'a>(containers: &'a [Container], target: &Target) -> anyhow::Result<&'a Container> {
    let found = containers.iter().find(|container| match target {
        Target::Index(index) => container.index == *index,
        Target::Name(name) => {
            container.session_name.as_deref() == Some(name) || container.name == *name
        }
    });
    match found {
        Some(container) => Ok(container),
        None => {
            let shown = match target {
                Target::Index(index) => index.to_string(),
                Target::Name(name) => name.clone(),
            };
            bail!("this repository has no container `{shown}`; `vz ls` lists them")
        }
    }
}

/// `vz attach` without a target: the only running container.
pub fn only_running(containers: &[Container]) -> anyhow::Result<&Container> {
    let running: Vec<&Container> = containers.iter().filter(|c| c.running).collect();
    match running[..] {
        [container] => Ok(container),
        [] => bail!("no container of this repository is running; `vz` or `vz new NAME` starts one"),
        _ => {
            let names: Vec<&str> = running.iter().map(|c| c.name.as_str()).collect();
            bail!(
                "several containers are running: {}; name one: `vz attach INDEX|NAME`",
                names.join(", ")
            )
        }
    }
}

/// A container is entered only with its own configuration: `vz new` never
/// lands in a trusted container.
pub fn check_config(container: &Container, config: Option<&str>) -> anyhow::Result<()> {
    if container.config.as_deref() == config {
        return Ok(());
    }
    let hint = match &container.config {
        Some(theirs) => format!("vz -c {theirs} attach {}", container.index),
        None => format!("vz attach {} without -c", container.index),
    };
    bail!(
        "{} runs {}; attach with `{hint}`",
        container.name,
        describe_config(container.config.as_deref())
    )
}

/// The configuration a container runs; without a label, `default`.
fn describe_config(config: Option<&str>) -> String {
    format!("configuration `{}`", config.unwrap_or(DEFAULT_CONFIG))
}

/// With `attach: true`, what `vz new` without a name joins: an unnamed
/// container of the same configuration, running, or kept; the lowest index,
/// running first.
pub fn to_join<'a>(containers: &'a [Container], config: Option<&str>) -> Option<&'a Container> {
    containers
        .iter()
        .filter(|c| c.session_name.is_none() && c.config.as_deref() == config)
        .filter(|c| c.running || c.persistent)
        .min_by_key(|c| (!c.running, c.index))
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    fn container(index: u32, config: Option<&str>, running: bool) -> Container {
        Container {
            name: container_name(index, "app", None),
            repo: PathBuf::from("/home/sally/repos/app"),
            index,
            session_name: None,
            config: config.map(str::to_owned),
            persistent: false,
            config_hash: "0123456789ab".to_owned(),
            chain: vec![],
            images: vec![],
            image: "debian:stable-slim".to_owned(),
            running,
            created: "2026-09-29 18:02".to_owned(),
        }
    }

    fn named(index: u32, name: &str) -> Container {
        Container {
            name: container_name(index, "app", Some(name)),
            session_name: Some(name.to_owned()),
            ..container(index, None, true)
        }
    }

    #[test]
    fn parse_inspect__vz_containers__read_from_labels_others_skipped() {
        let json = r#"[
          {
            "Name": "/vz-0-app",
            "Created": "2026-09-29T18:02:11.123456789Z",
            "State": { "Running": true, "Status": "running" },
            "Config": {
              "Image": "debian:stable-slim",
              "Labels": {
                "vz.repo": "/home/sally/repos/app", "vz.index": "0", "vz.name": "",
                "vz.config": "", "vz.persistent": "false", "vz.config_hash": "0123456789ab"
              }
            }
          },
          {
            "Name": "/vz-1-app-api",
            "Created": "2026-09-26T09:15:00Z",
            "State": { "Running": false },
            "Config": {
              "Image": "vz-app:abc",
              "Labels": {
                "vz.repo": "/home/sally/repos/app", "vz.index": "1", "vz.name": "api",
                "vz.config": "trusted", "vz.persistent": "true", "vz.config_hash": "ba9876543210"
              }
            }
          },
          {
            "Name": "/someone-elses",
            "Created": "2026-09-29T18:00:00Z",
            "State": { "Running": true },
            "Config": { "Image": "postgres:18", "Labels": null }
          }
        ]"#;

        let containers = parse_inspect(json).unwrap();

        let expected = vec![
            Container {
                created: "2026-09-29 18:02".to_owned(),
                ..container(0, None, true)
            },
            Container {
                name: "vz-1-app-api".to_owned(),
                session_name: Some("api".to_owned()),
                persistent: true,
                config_hash: "ba9876543210".to_owned(),
                image: "vz-app:abc".to_owned(),
                created: "2026-09-26 09:15".to_owned(),
                ..container(1, Some("trusted"), false)
            },
        ];
        assert_eq!(containers, expected);
    }

    #[test]
    fn parse_inspect__not_json__an_error() {
        let result = parse_inspect("Error: No such object");

        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("reading docker inspect's output"),
            "{message}"
        );
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn pairs__labels__every_label_even_when_empty() {
        let chain = strings(&[
            "default@~/.config/viz-shell/default.vz.yml",
            "ci@app.vz.yml",
        ]);
        let images = strings(&["vz-viz-shell:0123", "vz-app:4567"]);
        let labels = Labels {
            repo: Path::new("/r/app"),
            index: 2,
            session_name: None,
            config: Some("ci"),
            persistent: true,
            config_hash: "abc",
            chain: &chain,
            images: &images,
        };

        let pairs = labels.pairs();

        let expected = [
            ("vz.repo", "/r/app"),
            ("vz.index", "2"),
            ("vz.name", ""),
            ("vz.config", "ci"),
            ("vz.persistent", "true"),
            ("vz.config_hash", "abc"),
            (
                "vz.chain",
                "default@~/.config/viz-shell/default.vz.yml,ci@app.vz.yml",
            ),
            ("vz.image", "vz-viz-shell:0123,vz-app:4567"),
        ];
        let pairs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(label, value)| (label.as_str(), value.as_str()))
            .collect();
        assert_eq!(pairs, expected);
    }

    #[test]
    fn joined__entries_that_fit__joined_whole() {
        let cases = [
            (
                "two",
                strings(&["a@app.vz.yml", "b@app.vz.yml"]),
                "a@app.vz.yml,b@app.vz.yml",
            ),
            ("none", vec![], ""),
        ];
        for (case, entries, expected) in cases {
            let label = joined(&entries);

            assert_eq!(label, expected, "{case}");
        }
    }

    /// An entry `<name>@xxx…` of `len` bytes: its content does not matter.
    fn entry_of_len(name: &str, len: usize) -> String {
        format!("{name}@{}", "x".repeat(len - 2))
    }

    #[test]
    fn joined__entries_too_long__the_middle_cut_first_and_last_kept() {
        let cases = [
            (
                "four of 1500 bytes: two kept",
                ["a", "b", "c", "d"]
                    .map(|name| entry_of_len(name, 1500))
                    .to_vec(),
                &["a", "…", "d"][..],
            ),
            (
                "six of 800 bytes: five kept, the odd one at the head",
                ["a", "b", "c", "d", "e", "f"]
                    .map(|name| entry_of_len(name, 800))
                    .to_vec(),
                &["a", "b", "c", "…", "e", "f"][..],
            ),
        ];
        for (case, entries, expected) in cases {
            let label = joined(&entries);

            // Each kept entry by its name; the cut mark has no `@`.
            let names: Vec<&str> = label
                .split(',')
                .map(|entry| entry.split('@').next().unwrap())
                .collect();
            assert_eq!(names, expected, "{case}");
        }
    }

    #[test]
    fn joined__one_entry_too_long__only_the_cut_mark() {
        let entries = vec!["x".repeat(MAX_LABEL_LEN + 1)];

        let label = joined(&entries);

        assert_eq!(label, CUT);
    }

    /// One container whose chain and image labels hold two entries each.
    const WITH_CHAIN_AND_IMAGES: &str = r#"[{
            "Name": "/vz-0-app",
            "Created": "2026-09-29T18:02:11Z",
            "State": { "Running": true },
            "Config": {
              "Image": "vz-app:4567",
              "Labels": {
                "vz.repo": "/home/sally/repos/app", "vz.index": "0",
                "vz.chain": "default@~/.config/viz-shell/default.vz.yml,default@app.vz.yml",
                "vz.image": "vz-viz-shell:0123,vz-app:4567"
              }
            }
        }]"#;

    #[test]
    fn parse_inspect__chain_label__its_entries() {
        let containers = parse_inspect(WITH_CHAIN_AND_IMAGES).unwrap();

        let expected = strings(&[
            "default@~/.config/viz-shell/default.vz.yml",
            "default@app.vz.yml",
        ]);
        assert_eq!(containers[0].chain, expected);
    }

    #[test]
    fn parse_inspect__image_label__its_entries() {
        let containers = parse_inspect(WITH_CHAIN_AND_IMAGES).unwrap();

        assert_eq!(
            containers[0].images,
            strings(&["vz-viz-shell:0123", "vz-app:4567"])
        );
    }

    #[test]
    fn container_name__index_repository_and_name__vz_index_slug_name() {
        let cases = [
            ("unnamed", 0, "app", None, "vz-0-app"),
            ("named", 3, "app", Some("api"), "vz-3-app-api"),
            ("other characters", 1, "My_Repo.rs", None, "vz-1-my-repo-rs"),
            ("nothing left", 0, "___", None, "vz-0-repo"),
        ];
        for (case, index, repo, name, expected) in cases {
            let container = container_name(index, repo, name);

            assert_eq!(container, expected, "{case}");
        }
    }

    #[test]
    fn container_name__long_repository__cut_to_a_hostname_s_63_keeping_the_name() {
        let repository_of_80 = "a".repeat(80);

        let name = container_name(12, &repository_of_80, Some("apicoder"));

        assert_eq!(
            name,
            "vz-12-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-apicoder"
        );
    }

    #[test]
    fn check_session_name__lowercase_digits_dashes_from_a_letter__accepted() {
        for name in ["api", "apicoder", "web-2"] {
            let result = check_session_name(name);

            assert!(result.is_ok(), "{name}: {result:?}");
        }
    }

    #[test]
    fn check_session_name__anything_else__refused() {
        let too_long = "a".repeat(25);
        let cases = [
            ("empty", ""),
            ("a digit", "2"),
            ("from a digit", "2fa"),
            ("uppercase", "Api"),
            ("an underscore", "api_1"),
            ("a trailing dash", "api-"),
            ("25 characters", too_long.as_str()),
        ];
        for (case, name) in cases {
            let result = check_session_name(name);

            let message = format!("{:#}", result.unwrap_err());
            assert!(
                message.contains("is not a container name"),
                "{case}: {message}"
            );
        }
    }

    #[test]
    fn next_index__lowest_free() {
        let containers = [container(0, None, true), container(2, None, false)];
        let cases: [(&[Container], &[u32], u32); 4] = [
            (&[], &[], 0),
            (&containers, &[], 1),
            (&containers, &[1], 3),
            (&containers[1..], &[], 0),
        ];
        for (containers, taken, expected) in cases {
            let index = next_index(containers, taken);

            assert_eq!(index, expected, "{taken:?}");
        }
    }

    #[test]
    fn config_hash__any_text__twelve_hex_digits() {
        let hash = config_hash("image: debian\n");

        assert_eq!(hash.len(), CONFIG_HASH_LEN);
    }

    #[test]
    fn config_hash__same_text__same_hash() {
        let first = config_hash("image: debian\n");

        let second = config_hash("image: debian\n");

        assert_eq!(first, second);
    }

    #[test]
    fn config_hash__other_text__other_hash() {
        let debian = config_hash("image: debian\n");

        let alpine = config_hash("image: alpine\n");

        assert_ne!(debian, alpine);
    }

    #[test]
    fn target_parse__digits__an_index() {
        let target = Target::parse("0");

        assert_eq!(target, Target::Index(0));
    }

    #[test]
    fn target_parse__anything_else__a_name() {
        let target = Target::parse("api");

        assert_eq!(target, Target::Name("api".to_owned()));
    }

    #[test]
    fn find__index_session_name_or_full_name() {
        let containers = [container(0, None, true), named(1, "api")];
        let cases = [
            (Target::Index(0), "vz-0-app"),
            (Target::Index(1), "vz-1-app-api"),
            (Target::Name("api".to_owned()), "vz-1-app-api"),
            (Target::Name("vz-0-app".to_owned()), "vz-0-app"),
        ];
        for (target, expected) in cases {
            let found = find(&containers, &target).unwrap();

            assert_eq!(found.name, expected, "{target:?}");
        }
    }

    #[test]
    fn find__no_such_container__refused_pointing_at_vz_ls() {
        let containers = [container(0, None, true), named(1, "api")];

        let result = find(&containers, &Target::Index(7));

        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("this repository has no container `7`; `vz ls` lists them"),
            "{message}"
        );
    }

    #[test]
    fn only_running__one_running__that_one() {
        let containers = [container(0, None, false), container(1, None, true)];

        let found = only_running(&containers).unwrap();

        assert_eq!(found.index, 1);
    }

    #[test]
    fn only_running__none_running__refused() {
        let containers = [container(0, None, false)];

        let result = only_running(&containers);

        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("no container of this repository is running"),
            "{message}"
        );
    }

    #[test]
    fn only_running__several_running__refused_naming_them() {
        let containers = [container(0, None, true), container(1, None, true)];

        let result = only_running(&containers);

        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("several containers are running: vz-0-app, vz-1-app"),
            "{message}"
        );
    }

    #[test]
    fn check_config__its_own_config__accepted() {
        let cases = [
            (
                "trusted",
                container(0, Some("trusted"), true),
                Some("trusted"),
            ),
            ("the default", container(1, None, true), None),
        ];
        for (case, container, config) in cases {
            let result = check_config(&container, config);

            assert!(result.is_ok(), "{case}: {result:?}");
        }
    }

    #[test]
    fn check_config__other_config__refused_naming_the_command() {
        let cases = [
            (
                "trusted, entered without -c",
                container(0, Some("trusted"), true),
                None,
                "vz-0-app runs configuration `trusted`; attach with `vz -c trusted attach 0`",
            ),
            (
                "the default, entered with -c",
                container(1, None, true),
                Some("trusted"),
                "vz-1-app runs configuration `default`; attach with `vz attach 1 without -c`",
            ),
        ];
        for (case, container, config, expected) in cases {
            let result = check_config(&container, config);

            let message = format!("{:#}", result.unwrap_err());
            assert!(message.contains(expected), "{case}: {message}");
        }
    }

    /// Five containers of one repository: 0 kept (persistent, stopped),
    /// 1 trusted and running, 2 named `api`, 3 running, 4 stopped.
    fn five() -> [Container; 5] {
        [
            Container {
                persistent: true,
                ..container(0, None, false)
            },
            container(1, Some("trusted"), true),
            named(2, "api"),
            container(3, None, true),
            container(4, None, false),
        ]
    }

    /// A row: the case, the containers, the configuration asked for, and the
    /// index of the one joined.
    type JoinCase<'a> = (&'a str, &'a [Container], Option<&'a str>, Option<u32>);

    #[test]
    fn to_join__same_config_unnamed_running_or_kept() {
        let containers = five();
        let cases: [JoinCase; 4] = [
            (
                "a running one before a kept one",
                &containers,
                None,
                Some(3),
            ),
            ("a kept one alone", &containers[..1], None, Some(0)),
            (
                "only its own configuration",
                &containers,
                Some("trusted"),
                Some(1),
            ),
            ("none of that configuration", &containers, Some("ci"), None),
        ];
        for (case, containers, config, expected) in cases {
            let joined = to_join(containers, config);

            assert_eq!(joined.map(|c| c.index), expected, "{case}");
        }
    }
}
