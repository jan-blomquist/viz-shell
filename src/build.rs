//! Plans the image: a chain of pulls and builds, bottom first. The tag of a
//! built image carries a hash of the Dockerfile and its args, so an edit
//! anywhere else in `vz.yml` never rebuilds, and neither does a change to a
//! file the Dockerfile copies.
//!
//! A Dockerfile that declares `ARG VZ_UID` (or any of the user's build args)
//! gets the host user's value, so it can bake the user into the image; the
//! values join the hash, so such images are built per user.
//!
//! A Dockerfile that declares `ARG BASE` stacks: it is built on the image the
//! layers below it resolved to, passed as `BASE`, which joins its hash. One
//! without it, or an image reference, replaces what is below.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use docker_wrapper::BuildCommand;
use sha2::{Digest, Sha256};

use crate::config::{self, BuildSpec, ImageSource};
use crate::constants::{BASE_ARG, BUILT_IMAGE_PREFIX, CONTENT_HASH_LEN, FALLBACK_IMAGE_NAME};
use crate::user::User;

#[derive(Debug)]
pub struct BuildPlan {
    pub tag: String,
    dockerfile: PathBuf,
    context: PathBuf,
    args: BTreeMap<String, String>,
}

impl BuildPlan {
    /// `spec`'s args hold every build arg; the name comes from the
    /// Dockerfile's folder.
    fn new(spec: &BuildSpec, dockerfile_text: &str) -> Self {
        Self {
            tag: image_tag(&dir_name(&spec.dockerfile), dockerfile_text, &spec.args),
            dockerfile: spec.dockerfile.clone(),
            context: spec.context.clone(),
            args: spec.args.clone(),
        }
    }

    /// The image it is built on, when it stacks.
    pub fn base(&self) -> Option<&str> {
        self.args.get(BASE_ARG).map(String::as_str)
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

/// One image of the chain: a reference to pull, or a Dockerfile to build.
#[derive(Debug)]
pub enum ImageStep {
    Pull(String),
    Build(BuildPlan),
}

impl ImageStep {
    pub fn tag(&self) -> &str {
        match self {
            ImageStep::Pull(reference) => reference,
            ImageStep::Build(plan) => &plan.tag,
        }
    }
}

/// The steps that make the image of `chain` (every `image:` of the layers,
/// bottom first), bottom first; the last one's tag is the session's image.
/// Paths resolve against `config_dir`; the Dockerfiles are read.
pub fn plan(
    chain: &[ImageSource],
    config_dir: &Path,
    user: &User,
) -> anyhow::Result<Vec<ImageStep>> {
    let loaded = chain
        .iter()
        .map(|source| match source {
            ImageSource::Reference(reference) => Ok(Loaded::Reference(reference)),
            ImageSource::Build(spec) => {
                let dockerfile = config_dir.join(&spec.dockerfile);
                let text = std::fs::read_to_string(&dockerfile)
                    .with_context(|| format!("reading {}", dockerfile.display()))?;
                let spec = BuildSpec {
                    dockerfile,
                    context: config_dir.join(&spec.context),
                    args: spec.args.clone(),
                };
                Ok(Loaded::Build(spec, text))
            }
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(stack(&loaded, user))
}

/// An `image:` of the chain, its Dockerfile read.
enum Loaded<'a> {
    Reference(&'a str),
    /// Paths resolved; the Dockerfile's text.
    Build(BuildSpec, String),
}

/// Walks the chain bottom-up: a reference, or a Dockerfile that does not
/// stack, starts over; one that stacks gets `BASE`, the tag of the step
/// below, when there is one. `vz.yml` args win over the user's and `BASE`
/// alike: a `BASE` set there names the base, so the step starts over.
fn stack(chain: &[Loaded], user: &User) -> Vec<ImageStep> {
    let mut steps: Vec<ImageStep> = Vec::new();
    for source in chain {
        let step = match source {
            Loaded::Reference(reference) => {
                steps.clear();
                ImageStep::Pull(config::with_default_tag(reference))
            }
            Loaded::Build(spec, text) => {
                let mut args = identity_args(text, user);
                let stacks = declares_base(text) && !spec.args.contains_key(BASE_ARG);
                match steps.last() {
                    Some(below) if stacks => {
                        args.insert(BASE_ARG.to_owned(), below.tag().to_owned());
                    }
                    _ => steps.clear(),
                }
                args.extend(spec.args.clone());
                let spec = BuildSpec {
                    args,
                    ..spec.clone()
                };
                ImageStep::Build(BuildPlan::new(&spec, text))
            }
        };
        steps.push(step);
    }
    steps
}

/// Whether the Dockerfile declares `ARG BASE`, so it stacks on the image
/// below it; an unreadable one does not.
pub fn stacks(dockerfile: &Path) -> bool {
    std::fs::read_to_string(dockerfile).is_ok_and(|text| declares_base(&text))
}

fn declares_base(dockerfile_text: &str) -> bool {
    declared_args(dockerfile_text).contains(BASE_ARG)
}

/// One line per image of the chain, bottom first, with what each does to
/// the ones below: `debian:stable-slim`, `~/tools/Dockerfile (ARG BASE:
/// stacks)`, `Dockerfile (replaces)`.
pub fn describe(chain: &[ImageSource], config_dir: &Path, home: &Path) -> Vec<String> {
    chain
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let (name, stacking) = match source {
                ImageSource::Reference(reference) => (reference.clone(), false),
                ImageSource::Build(spec) => {
                    let dockerfile = config_dir.join(&spec.dockerfile);
                    let stacking = !spec.args.contains_key(BASE_ARG) && stacks(&dockerfile);
                    (config::tilde(&dockerfile, home), stacking)
                }
            };
            match (index, stacking) {
                (0, false) => name,
                (0, true) => format!("{name} (ARG BASE: its default)"),
                (_, false) => format!("{name} (replaces)"),
                (_, true) => format!("{name} (ARG BASE: stacks)"),
            }
        })
        .collect()
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

/// `vz-<the Dockerfile's folder>:<content hash>`.
fn image_tag(dir_name: &str, dockerfile_text: &str, args: &BTreeMap<String, String>) -> String {
    format!(
        "{BUILT_IMAGE_PREFIX}{}:{}",
        image_name(dir_name),
        content_hash(dockerfile_text, args)
    )
}

/// The name of the folder the Dockerfile is in: one Dockerfile makes one
/// image, whichever repository uses it.
fn dir_name(dockerfile: &Path) -> String {
    dockerfile
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// An image name allows lowercase letters, digits and inner separators;
/// everything else becomes `-`.
fn image_name(dir_name: &str) -> String {
    let name: String = dir_name
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
    fn plan__paths_relative_to_config_dir__vz_yml_args_win_over_identity() {
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

        let steps = plan(&[ImageSource::Build(spec)], config_dir.path(), &sally()).unwrap();

        let [ImageStep::Build(plan)] = steps.as_slice() else {
            panic!("{steps:?}");
        };
        assert_eq!(plan.dockerfile, config_dir.path().join("Dockerfile"));
        assert_eq!(plan.context, config_dir.path().join("."));
        assert_eq!(plan.args, args(&[("VZ_UID", "4242")]));
    }

    #[test]
    fn stacks__dockerfile__declares_arg_base_or_not() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("ARG BASE\nFROM ${BASE}\n", true),
            ("ARG BASE=debian:stable-slim\nFROM ${BASE}\n", true),
            ("arg BASE\nFROM ${BASE}\n", true),
            ("FROM debian:stable-slim\n", false),
            ("ARG OTHER\nFROM debian:stable-slim\n", false),
        ];
        for (text, expected) in cases {
            let dockerfile = dir.path().join("Dockerfile");
            std::fs::write(&dockerfile, text).unwrap();

            assert_eq!(stacks(&dockerfile), expected, "{text}");
        }
    }

    #[test]
    fn stacks__no_dockerfile__false() {
        assert!(!stacks(Path::new("/nonexistent/Dockerfile")));
    }

    const STACKING: &str = "ARG BASE=debian:stable-slim\nFROM ${BASE}\n";
    const REPLACING: &str = "FROM debian:stable-slim\n";

    /// A Dockerfile of the chain at `/repos/<dir>/Dockerfile`.
    fn build(dir: &str, text: &str) -> Loaded<'static> {
        let spec = BuildSpec {
            dockerfile: PathBuf::from(format!("/repos/{dir}/Dockerfile")),
            context: PathBuf::from(format!("/repos/{dir}")),
            args: BTreeMap::new(),
        };
        Loaded::Build(spec, text.to_owned())
    }

    /// Each step as `pull <ref>` or `build <name> [on <base>]`.
    fn shape(steps: &[ImageStep]) -> Vec<String> {
        steps
            .iter()
            .map(|step| match step {
                ImageStep::Pull(reference) => format!("pull {reference}"),
                ImageStep::Build(plan) => {
                    let (name, _) = plan.tag.split_once(':').unwrap();
                    match plan.base() {
                        Some(base) => format!("build {name} on {base}"),
                        None => format!("build {name}"),
                    }
                }
            })
            .collect()
    }

    #[test]
    fn stack__chains__steps_that_contribute_bottom_first() {
        let debian = || Loaded::Reference("debian");
        let cases = [
            (vec![debian()], vec!["pull debian:latest"]),
            (
                vec![debian(), build("tools", STACKING)],
                vec!["pull debian:latest", "build vz-tools on debian:latest"],
            ),
            (
                vec![debian(), build("alone", REPLACING)],
                vec!["build vz-alone"],
            ),
            (vec![build("tools", STACKING)], vec!["build vz-tools"]),
            (
                vec![build("tools", STACKING), debian()],
                vec!["pull debian:latest"],
            ),
            (
                vec![
                    build("base", REPLACING),
                    build("tools", STACKING),
                    build("alone", REPLACING),
                ],
                vec!["build vz-alone"],
            ),
        ];
        for (chain, expected) in cases {
            let steps = stack(&chain, &sally());

            assert_eq!(shape(&steps), expected);
        }
    }

    #[test]
    fn stack__three_dockerfiles_stacking__each_on_the_previous_tag() {
        let chain = [
            build("base", REPLACING),
            build("tools", STACKING),
            build("agents", STACKING),
        ];

        let steps = stack(&chain, &sally());

        let tags: Vec<&str> = steps.iter().map(ImageStep::tag).collect();
        let bases: Vec<Option<&str>> = steps
            .iter()
            .map(|step| match step {
                ImageStep::Build(plan) => plan.base(),
                ImageStep::Pull(_) => panic!("{step:?}"),
            })
            .collect();
        assert_eq!(bases, [None, Some(tags[0]), Some(tags[1])]);
    }

    #[test]
    fn stack__base_dockerfile_edit__changes_the_top_tag() {
        let top = |base_text: &str| {
            let chain = [build("base", base_text), build("tools", STACKING)];
            stack(&chain, &sally()).last().unwrap().tag().to_owned()
        };

        let before = top(REPLACING);

        let after = top("FROM debian:stable-slim\nRUN true\n");

        assert_ne!(before, after);
    }

    #[test]
    fn stack__base_set_in_vz_yml__replaces_what_is_below() {
        let Loaded::Build(mut spec, text) = build("tools", STACKING) else {
            unreachable!()
        };
        spec.args = args(&[("BASE", "alpine:3")]);
        let chain = [Loaded::Reference("debian"), Loaded::Build(spec, text)];

        let steps = stack(&chain, &sally());

        assert_eq!(shape(&steps), ["build vz-tools on alpine:3"]);
    }

    #[test]
    fn describe__chain__each_image_with_what_it_does() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [("stacking", STACKING), ("replacing", REPLACING)] {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        let build = |name: &str| {
            ImageSource::Build(BuildSpec {
                dockerfile: PathBuf::from(name),
                context: PathBuf::from("."),
                args: BTreeMap::new(),
            })
        };
        let chain = [
            build("stacking"),
            ImageSource::Reference("debian".to_owned()),
            build("stacking"),
            build("replacing"),
        ];

        let lines = describe(&chain, dir.path(), dir.path());

        assert_eq!(
            lines,
            [
                "~/stacking (ARG BASE: its default)",
                "debian (replaces)",
                "~/stacking (ARG BASE: stacks)",
                "~/replacing (replaces)",
            ]
        );
    }

    #[test]
    fn dir_name__dockerfile_paths() {
        let cases = [
            ("/home/sally/repos/app/Dockerfile", "app"),
            ("/home/sally/repos/app/images/tools/Dockerfile", "tools"),
            ("/Dockerfile", ""),
        ];
        for (dockerfile, expected) in cases {
            assert_eq!(dir_name(Path::new(dockerfile)), expected, "{dockerfile}");
        }
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
        let plan = BuildPlan::new(&spec, DOCKERFILE);

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
