//! A repository's containers: their names, the labels that identify them,
//! and which one a command means. Labels are the store: vz finds containers
//! by label, never by parsing names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::constants::{
    CONFIG_LABEL, CONTAINER_PREFIX, INDEX_LABEL, NAME_LABEL, PERSISTENT_LABEL, PROFILE_LABEL,
    REPO_LABEL,
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
    pub profile: Option<String>,
    pub persistent: bool,
    /// The configuration's hash when it was created.
    pub config: String,
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
            profile: label(PROFILE_LABEL),
            persistent: label(PERSISTENT_LABEL).as_deref() == Some("true"),
            config: label(CONFIG_LABEL).unwrap_or_default(),
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

/// The labels a new container carries.
pub fn labels(
    repo: &Path,
    index: u32,
    session_name: Option<&str>,
    profile: Option<&str>,
    persistent: bool,
    config: &str,
) -> Vec<(String, String)> {
    [
        (REPO_LABEL, repo.display().to_string()),
        (INDEX_LABEL, index.to_string()),
        (NAME_LABEL, session_name.unwrap_or_default().to_owned()),
        (PROFILE_LABEL, profile.unwrap_or_default().to_owned()),
        (PERSISTENT_LABEL, persistent.to_string()),
        (CONFIG_LABEL, config.to_owned()),
    ]
    .into_iter()
    .map(|(label, value)| (label.to_owned(), value))
    .collect()
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

/// A container is entered only with its own profile: a plain `vz` never
/// lands in a trusted container.
pub fn check_profile(container: &Container, profile: Option<&str>) -> anyhow::Result<()> {
    if container.profile.as_deref() == profile {
        return Ok(());
    }
    let hint = match &container.profile {
        Some(theirs) => format!("vz --profile {theirs} attach {}", container.index),
        None => format!("vz attach {} without --profile", container.index),
    };
    bail!(
        "{} runs {}; attach with `{hint}`",
        container.name,
        describe_profile(container.profile.as_deref())
    )
}

fn describe_profile(profile: Option<&str>) -> String {
    match profile {
        Some(profile) => format!("profile `{profile}`"),
        None => "no profile".to_owned(),
    }
}

/// With `attach: true`, what a plain `vz` joins: an unnamed container of the
/// same profile, running, or kept; the lowest index, running first.
pub fn to_join<'a>(containers: &'a [Container], profile: Option<&str>) -> Option<&'a Container> {
    containers
        .iter()
        .filter(|c| c.session_name.is_none() && c.profile.as_deref() == profile)
        .filter(|c| c.running || c.persistent)
        .min_by_key(|c| (!c.running, c.index))
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    fn container(index: u32, profile: Option<&str>, running: bool) -> Container {
        Container {
            name: container_name(index, "app", None),
            repo: PathBuf::from("/home/sally/repos/app"),
            index,
            session_name: None,
            profile: profile.map(str::to_owned),
            persistent: false,
            config: "0123456789ab".to_owned(),
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
                "vz.profile": "", "vz.persistent": "false", "vz.config": "0123456789ab"
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
                "vz.profile": "trusted", "vz.persistent": "true", "vz.config": "ba9876543210"
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
                config: "ba9876543210".to_owned(),
                image: "vz-app:abc".to_owned(),
                created: "2026-09-26 09:15".to_owned(),
                ..container(1, Some("trusted"), false)
            },
        ];
        assert_eq!(containers, expected);
    }

    #[test]
    fn parse_inspect__not_json__an_error() {
        assert!(parse_inspect("Error: No such object").is_err());
    }

    #[test]
    fn labels__session__every_label_even_when_empty() {
        let labels = labels(Path::new("/r/app"), 2, None, Some("ci"), true, "abc");

        let expected = [
            ("vz.repo", "/r/app"),
            ("vz.index", "2"),
            ("vz.name", ""),
            ("vz.profile", "ci"),
            ("vz.persistent", "true"),
            ("vz.config", "abc"),
        ];
        let labels: Vec<(&str, &str)> = labels
            .iter()
            .map(|(label, value)| (label.as_str(), value.as_str()))
            .collect();
        assert_eq!(labels, expected);
    }

    #[test]
    fn container_name__cases() {
        let long = "a".repeat(80);
        let cases = [
            (0, "app", None, "vz-0-app"),
            (3, "app", Some("api"), "vz-3-app-api"),
            (1, "My_Repo.rs", None, "vz-1-my-repo-rs"),
            (0, "___", None, "vz-0-repo"),
        ];
        for (index, repo, name, expected) in cases {
            assert_eq!(container_name(index, repo, name), expected, "{repo}");
        }
        let name = container_name(12, &long, Some("apicoder"));
        assert_eq!(name.len(), 63, "{name}");
        assert!(name.ends_with("a-apicoder"), "{name}");
    }

    #[test]
    fn check_session_name__cases() {
        for name in ["api", "apicoder", "web-2"] {
            assert!(check_session_name(name).is_ok(), "{name}");
        }
        for name in ["", "2", "2fa", "Api", "api_1", "api-", &"a".repeat(25)] {
            assert!(check_session_name(name).is_err(), "{name}");
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
            assert_eq!(next_index(containers, taken), expected, "{taken:?}");
        }
    }

    #[test]
    fn config_hash__same_text_same_hash_else_another() {
        let hash = config_hash("image: debian\n");

        assert_eq!(hash.len(), 12);
        assert_eq!(hash, config_hash("image: debian\n"));
        assert_ne!(hash, config_hash("image: alpine\n"));
    }

    #[test]
    fn target_parse__digits_index_else_name() {
        assert_eq!(Target::parse("0"), Target::Index(0));
        assert_eq!(Target::parse("api"), Target::Name("api".to_owned()));
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
            assert_eq!(find(&containers, &target).unwrap().name, expected);
        }
        let missing = find(&containers, &Target::Index(7))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("vz ls"), "{missing}");
    }

    #[test]
    fn only_running__one_none_or_several() {
        let one = [container(0, None, false), container(1, None, true)];
        let several = [container(0, None, true), container(1, None, true)];

        assert_eq!(only_running(&one).unwrap().index, 1);
        assert!(only_running(&one[..1]).is_err());
        let error = only_running(&several).unwrap_err().to_string();
        assert!(error.contains("vz-0-app, vz-1-app"), "{error}");
    }

    #[test]
    fn check_profile__other_profile__refused_naming_the_command() {
        let trusted = container(0, Some("trusted"), true);
        let plain = container(1, None, true);

        assert!(check_profile(&trusted, Some("trusted")).is_ok());
        assert!(check_profile(&plain, None).is_ok());
        let error = check_profile(&trusted, None).unwrap_err().to_string();
        assert!(error.contains("vz --profile trusted attach 0"), "{error}");
        let error = check_profile(&plain, Some("trusted"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("vz attach 1 without --profile"), "{error}");
    }

    #[test]
    fn to_join__same_profile_unnamed_running_or_kept() {
        let kept = Container {
            persistent: true,
            ..container(0, None, false)
        };
        let containers = [
            kept.clone(),
            container(1, Some("trusted"), true),
            named(2, "api"),
            container(3, None, true),
            container(4, None, false),
        ];

        assert_eq!(to_join(&containers, None).map(|c| c.index), Some(3));
        assert_eq!(to_join(&containers[..1], None), Some(&kept));
        assert_eq!(
            to_join(&containers, Some("trusted")).map(|c| c.index),
            Some(1)
        );
        assert_eq!(to_join(&containers, Some("ci")), None);
    }
}
