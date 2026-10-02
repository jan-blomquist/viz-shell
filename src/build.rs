//! Plans the image: a chain of pulls and builds, bottom first. The tag of a
//! built image carries a hash of the Dockerfile and its args, so an edit
//! anywhere else in a configuration file never rebuilds, and neither does a
//! change to a file the Dockerfile copies.
//!
//! A Dockerfile that declares `ARG VZ_UID` (or any of the user's build args)
//! gets the host user's value, so it can bake the user into the image; the
//! values join the hash, so such images are built per user.
//!
//! A Dockerfile that declares `ARG BASE` stacks: it is built on the image the
//! configurations before it in the chain resolved to, passed as `BASE`, which joins
//! its hash. `ARG BASE=<default>` builds alone on its default when nothing is
//! below; `ARG BASE` without one requires an image below, else it is refused.
//! One without it, or an image reference, replaces what is below.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use docker_wrapper::BuildCommand;
use sha2::{Digest, Sha256};

use crate::config::{self, BuildSpec, ImageLayer, ImageSource};
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

/// The steps that make the image of `chain` (every `image:` of the
/// configuration chain, bottom first), bottom first; the last one's tag is
/// the session's image. Paths resolve against `config_dir`, the repository
/// root; the Dockerfiles are read.
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
    stack(&loaded, user, |dockerfile| {
        config::shown(dockerfile, config_dir, &user.home)
    })
}

/// An `image:` of the chain, its Dockerfile read.
enum Loaded<'a> {
    Reference(&'a str),
    /// Paths resolved; the Dockerfile's text.
    Build(BuildSpec, String),
}

/// Walks the chain bottom-up: a reference, or a Dockerfile that does not
/// stack, starts over; one that stacks gets `BASE`, the tag of the step
/// below. Configured args win over the user's and `BASE` alike: a `BASE` set
/// there names the base, so the step starts over. `shown` names a
/// Dockerfile in a refusal.
fn stack(
    chain: &[Loaded],
    user: &User,
    shown: impl Fn(&Path) -> String,
) -> anyhow::Result<Vec<ImageStep>> {
    let mut steps: Vec<ImageStep> = Vec::new();
    for source in chain {
        let step = match source {
            Loaded::Reference(reference) => {
                steps.clear();
                ImageStep::Pull(config::with_default_tag(reference))
            }
            Loaded::Build(spec, text) => {
                let mut args = identity_args(text, user);
                let placement = place(
                    base_arg(text),
                    spec.args.contains_key(BASE_ARG),
                    !steps.is_empty(),
                    &shown(&spec.dockerfile),
                )?;
                match (placement, steps.last()) {
                    (Placement::Stacks { .. }, Some(below)) => {
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
    Ok(steps)
}

/// How a Dockerfile declares `BASE` before its first `FROM`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BaseArg {
    /// No `ARG BASE`: it builds on what its `FROM` names.
    None,
    /// `ARG BASE=<default>`: on the image below it, else on its default.
    Optional,
    /// `ARG BASE`, or `ARG BASE=` with an empty default: on the image below
    /// it, which there must be.
    Required,
}

/// The `BASE` declaration of a Dockerfile. Only an `ARG` before the first
/// `FROM` feeds a `FROM`; one after it belongs to a build stage, so it is
/// no base. The instruction's case does not matter, the argument's does.
pub fn base_arg(dockerfile_text: &str) -> BaseArg {
    for line in dockerfile_text.lines() {
        let Some((instruction, rest)) = line.trim_start().split_once(char::is_whitespace) else {
            continue;
        };
        if instruction.eq_ignore_ascii_case("FROM") {
            break;
        }
        if !instruction.eq_ignore_ascii_case("ARG") {
            continue;
        }
        for declaration in rest.split_whitespace() {
            match declaration.split_once('=') {
                None if declaration == BASE_ARG => return BaseArg::Required,
                Some((BASE_ARG, "")) => return BaseArg::Required,
                Some((BASE_ARG, _)) => return BaseArg::Optional,
                _ => {}
            }
        }
    }
    BaseArg::None
}

/// The `BASE` declaration of a Dockerfile on disk; an unreadable one has none.
fn base_arg_of(dockerfile: &Path) -> BaseArg {
    std::fs::read_to_string(dockerfile).map_or(BaseArg::None, |text| base_arg(&text))
}

/// What a Dockerfile's image does to the images before it in the chain.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Placement {
    /// Starts over: a `FROM` of its own, or `BASE` set in the configuration.
    Replaces,
    /// Nothing before it: builds on its `ARG BASE` default.
    ItsDefault,
    /// Built on the image before it, passed as `BASE`.
    Stacks { required: bool },
}

/// Where a Dockerfile declaring `base` lands, given whether the
/// configuration sets `BASE` itself and whether an image comes before it.
/// A required `BASE` with nothing before it is refused, naming `dockerfile`.
fn place(
    base: BaseArg,
    base_configured: bool,
    image_before: bool,
    dockerfile: &str,
) -> anyhow::Result<Placement> {
    if base_configured {
        return Ok(Placement::Replaces);
    }
    match (base, image_before) {
        (BaseArg::None, _) => Ok(Placement::Replaces),
        (BaseArg::Optional, false) => Ok(Placement::ItsDefault),
        (BaseArg::Optional, true) => Ok(Placement::Stacks { required: false }),
        (BaseArg::Required, true) => Ok(Placement::Stacks { required: true }),
        (BaseArg::Required, false) => {
            bail!("`{dockerfile}` requires BASE: no image before it in the chain")
        }
    }
}

/// One line per image of the chain, bottom first, with the configuration and
/// the file that set it, and what it does to the ones below:
/// `debian:trixie (base, ~/.config/viz-shell/base.vz.yml)`, `Dockerfile
/// (default, vz.yml, ARG BASE: required, stacks)`, `gpu.Dockerfile (gpu,
/// gpu.vz.yml, replaces)`. Paths in `repo_root` are named relative to
/// it; others by `~/…` or in full. A Dockerfile that requires `BASE` with no
/// image before it is refused.
pub fn describe(
    chain: &[ImageLayer],
    repo_root: &Path,
    home: &Path,
) -> anyhow::Result<Vec<String>> {
    chain
        .iter()
        .enumerate()
        .map(|(index, image)| {
            let set_by = format!(
                "{}, {}",
                image.config,
                config::shown(&image.file, repo_root, home)
            );
            let (name, placement) = match &image.source {
                ImageSource::Reference(reference) => (reference.clone(), Placement::Replaces),
                ImageSource::Build(spec) => {
                    let name = config::shown(&spec.dockerfile, repo_root, home);
                    let placement = place(
                        base_arg_of(&spec.dockerfile),
                        spec.args.contains_key(BASE_ARG),
                        index > 0,
                        &name,
                    )?;
                    (name, placement)
                }
            };
            Ok(match (index, placement) {
                (0, Placement::Replaces) => format!("{name} ({set_by})"),
                (_, Placement::Replaces) => format!("{name} ({set_by}, replaces)"),
                (_, Placement::ItsDefault) => format!("{name} ({set_by}, ARG BASE: its default)"),
                (_, Placement::Stacks { required: false }) => {
                    format!("{name} ({set_by}, ARG BASE: stacks)")
                }
                (_, Placement::Stacks { required: true }) => {
                    format!("{name} ({set_by}, ARG BASE: required, stacks)")
                }
            })
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
    fn declared_args__arg_forms__every_name_declared_before_or_after_from() {
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

    /// The plan of a build step; a pull is a test failure.
    fn build_plan(step: &ImageStep) -> &BuildPlan {
        let ImageStep::Build(plan) = step else {
            panic!("a build, not {step:?}");
        };
        plan
    }

    /// A configuration folder whose `Dockerfile` declares `VZ_UID`, which
    /// its configuration sets to 4242: the plan of that one image.
    fn plan_with_configured_uid() -> (tempfile::TempDir, Vec<ImageStep>) {
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
        (config_dir, steps)
    }

    #[test]
    fn plan__relative_paths__resolved_against_the_config_dir() {
        let (config_dir, steps) = plan_with_configured_uid();

        let plan = build_plan(&steps[0]);

        let expected = (
            config_dir.path().join("Dockerfile"),
            config_dir.path().join("."),
        );
        assert_eq!((plan.dockerfile.clone(), plan.context.clone()), expected);
    }

    #[test]
    fn plan__an_identity_arg_set_in_the_configuration__the_configured_value_wins() {
        let (_config_dir, steps) = plan_with_configured_uid();

        let plan = build_plan(&steps[0]);

        assert_eq!(plan.args, args(&[("VZ_UID", "4242")]));
    }

    #[test]
    fn base_arg__base_without_a_default__required() {
        let cases = [
            ("bare", "ARG BASE\nFROM ${BASE}\n"),
            ("an empty default", "ARG BASE=\nFROM ${BASE}\n"),
            ("a lowercase instruction", "arg BASE\nFROM ${BASE}\n"),
            ("two spaces", "ARG  BASE\nFROM ${BASE}\n"),
        ];
        for (case, text) in cases {
            let base = base_arg(text);

            assert_eq!(base, BaseArg::Required, "{case}");
        }
    }

    #[test]
    fn base_arg__base_with_a_default__optional() {
        let cases = [
            ("a default", "ARG BASE=debian:stable-slim\nFROM ${BASE}\n"),
            ("after a tab", "ARG\tBASE=x\nFROM ${BASE}\n"),
            ("second on the line", "ARG OTHER BASE=x\nFROM ${BASE}\n"),
        ];
        for (case, text) in cases {
            let base = base_arg(text);

            assert_eq!(base, BaseArg::Optional, "{case}");
        }
    }

    #[test]
    fn base_arg__no_base_before_the_first_from__none() {
        let cases = [
            ("the argument in lowercase", "arg base\nFROM ${base}\n"),
            ("another argument", "ARG OTHER\nFROM debian:stable-slim\n"),
            ("commented out", "# ARG BASE\nFROM debian:stable-slim\n"),
            (
                "after FROM: a stage's",
                "FROM debian:stable-slim\nARG BASE=x\n",
            ),
            ("no ARG at all", "FROM debian:stable-slim\n"),
        ];
        for (case, text) in cases {
            let base = base_arg(text);

            assert_eq!(base, BaseArg::None, "{case}");
        }
    }

    #[test]
    fn base_arg_of__no_dockerfile__none() {
        let base = base_arg_of(Path::new("/nonexistent/Dockerfile"));

        assert_eq!(base, BaseArg::None);
    }

    const STACKING: &str = "ARG BASE=debian:stable-slim\nFROM ${BASE}\n";
    const REQUIRED: &str = "ARG BASE\nFROM ${BASE}\n";
    const REPLACING: &str = "FROM debian:stable-slim\n";

    /// The chain stacked for Sally, Dockerfiles named by their path.
    fn stacked(chain: &[Loaded]) -> anyhow::Result<Vec<ImageStep>> {
        stack(chain, &sally(), |dockerfile| {
            dockerfile.display().to_string()
        })
    }

    /// A Dockerfile of the chain at `/repos/<dir>/Dockerfile`.
    fn build(dir: &str, text: &str) -> Loaded<'static> {
        build_with_args(dir, text, &[])
    }

    /// [`build`], its configuration setting `configured` build args.
    fn build_with_args(dir: &str, text: &str, configured: &[(&str, &str)]) -> Loaded<'static> {
        let spec = BuildSpec {
            dockerfile: PathBuf::from(format!("/repos/{dir}/Dockerfile")),
            context: PathBuf::from(format!("/repos/{dir}")),
            args: args(configured),
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
            (
                "a reference alone",
                vec![debian()],
                vec!["pull debian:latest"],
            ),
            (
                "an optional BASE on a reference",
                vec![debian(), build("tools", STACKING)],
                vec!["pull debian:latest", "build vz-tools on debian:latest"],
            ),
            (
                "no BASE on a reference",
                vec![debian(), build("alone", REPLACING)],
                vec!["build vz-alone"],
            ),
            (
                "an optional BASE with nothing below",
                vec![build("tools", STACKING)],
                vec!["build vz-tools"],
            ),
            (
                "a required BASE on a reference",
                vec![debian(), build("overlay", REQUIRED)],
                vec!["pull debian:latest", "build vz-overlay on debian:latest"],
            ),
            (
                "a reference on a Dockerfile",
                vec![build("tools", STACKING), debian()],
                vec!["pull debian:latest"],
            ),
            (
                "no BASE on two stacked",
                vec![
                    build("base", REPLACING),
                    build("tools", STACKING),
                    build("alone", REPLACING),
                ],
                vec!["build vz-alone"],
            ),
        ];
        for (case, chain, expected) in cases {
            let steps = stacked(&chain).unwrap();

            assert_eq!(shape(&steps), expected, "{case}");
        }
    }

    #[test]
    fn stack__base_required_with_nothing_before_it__refused_naming_the_dockerfile() {
        let chain = [build("overlay", REQUIRED)];

        let message = format!("{:#}", stacked(&chain).unwrap_err());

        assert!(
            message.contains(
                "`/repos/overlay/Dockerfile` requires BASE: no image before it in the chain"
            ),
            "{message}"
        );
    }

    #[test]
    fn stack__three_dockerfiles_stacking__each_on_the_previous_tag() {
        let chain = [
            build("base", REPLACING),
            build("tools", STACKING),
            build("agents", STACKING),
        ];

        let steps = stacked(&chain).unwrap();

        // The rule itself links actual tags: each step's BASE is the tag
        // below it, whatever its hash.
        let tags: Vec<&str> = steps.iter().map(ImageStep::tag).collect();
        let bases: Vec<Option<&str>> = steps.iter().map(|step| build_plan(step).base()).collect();
        assert_eq!(bases, [None, Some(tags[0]), Some(tags[1])]);
    }

    #[test]
    fn stack__base_dockerfile_edit__changes_the_top_tag() {
        let top = |base_text: &str| {
            let chain = [build("base", base_text), build("tools", STACKING)];
            stacked(&chain).unwrap().last().unwrap().tag().to_owned()
        };

        let before = top(REPLACING);

        let after = top("FROM debian:stable-slim\nRUN true\n");

        assert_ne!(before, after);
    }

    #[test]
    fn stack__base_set_in_the_configuration__replaces_what_is_below() {
        let chain = [
            Loaded::Reference("debian"),
            build_with_args("tools", STACKING, &[("BASE", "alpine:3")]),
        ];

        let steps = stacked(&chain).unwrap();

        assert_eq!(shape(&steps), ["build vz-tools on alpine:3"]);
    }

    /// An `image:` set by `config` in `/repo/app.vz.yml`.
    fn image(config: &str, source: ImageSource) -> ImageLayer {
        ImageLayer {
            config: config.to_owned(),
            file: PathBuf::from("/repo/app.vz.yml"),
            source,
        }
    }

    #[test]
    fn describe__chain__each_image_with_its_configuration_file_and_what_it_does() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stacking"), STACKING).unwrap();
        std::fs::write(dir.path().join("required"), REQUIRED).unwrap();
        std::fs::write(dir.path().join("replacing"), REPLACING).unwrap();
        let build = |config: &str, name: &str| {
            let spec = BuildSpec {
                dockerfile: dir.path().join(name),
                context: dir.path().to_owned(),
                args: BTreeMap::new(),
            };
            image(config, ImageSource::Build(spec))
        };
        let chain = [
            build("default", "stacking"),
            image("a", ImageSource::Reference("debian".to_owned())),
            build("b", "stacking"),
            build("c", "required"),
            build("d", "replacing"),
        ];

        let lines = describe(&chain, dir.path(), Path::new("/home/sally")).unwrap();

        assert_eq!(
            lines,
            [
                "stacking (default, /repo/app.vz.yml, ARG BASE: its default)",
                "debian (a, /repo/app.vz.yml, replaces)",
                "stacking (b, /repo/app.vz.yml, ARG BASE: stacks)",
                "required (c, /repo/app.vz.yml, ARG BASE: required, stacks)",
                "replacing (d, /repo/app.vz.yml, replaces)",
            ]
        );
    }

    #[test]
    fn describe__base_required_first_in_the_chain__refused_naming_it_as_the_header_does() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".vz.Dockerfile"), REQUIRED).unwrap();
        let spec = BuildSpec {
            dockerfile: dir.path().join(".vz.Dockerfile"),
            context: dir.path().to_owned(),
            args: BTreeMap::new(),
        };
        let chain = [image("default", ImageSource::Build(spec))];

        let result = describe(&chain, dir.path(), Path::new("/home/sally"));

        let message = format!("{:#}", result.unwrap_err());

        assert!(
            message.contains("`.vz.Dockerfile` requires BASE: no image before it in the chain"),
            "{message}"
        );
    }

    #[test]
    fn describe__files_in_the_repository_and_the_home__shown_relative_and_with_tilde() {
        let spec = BuildSpec {
            dockerfile: PathBuf::from("/home/sally/.config/viz-shell/vz-debian-trixie.Dockerfile"),
            context: PathBuf::from("/home/sally/.config/viz-shell"),
            args: BTreeMap::new(),
        };
        let chain = [ImageLayer {
            config: "default".to_owned(),
            file: PathBuf::from("/home/sally/repos/app/app.vz.yml"),
            source: ImageSource::Build(spec),
        }];

        let lines = describe(
            &chain,
            Path::new("/home/sally/repos/app"),
            Path::new("/home/sally"),
        )
        .unwrap();

        assert_eq!(
            lines,
            ["~/.config/viz-shell/vz-debian-trixie.Dockerfile (default, app.vz.yml)"]
        );
    }

    #[test]
    fn dir_name__dockerfile_paths__the_folder_holding_it() {
        let cases = [
            ("/home/sally/repos/app/Dockerfile", "app"),
            ("/home/sally/repos/app/images/tools/Dockerfile", "tools"),
            ("/Dockerfile", ""),
        ];
        for (dockerfile, expected) in cases {
            let name = dir_name(Path::new(dockerfile));

            assert_eq!(name, expected, "{dockerfile}");
        }
    }

    #[test]
    fn image_name__directory_names__lowercase_other_characters_dashed_and_trimmed() {
        let cases = [
            ("vz", "vz"),
            ("viz-shell", "viz-shell"),
            ("My Repo", "my-repo"),
            ("_private_", "private"),
            ("über", "ber"),
        ];
        for (dir_name, expected) in cases {
            let name = image_name(dir_name);

            assert_eq!(name, expected, "dir_name: {dir_name}");
        }
    }

    #[test]
    fn image_name__nothing_left__the_fallback_name() {
        let name = image_name("...");

        assert_eq!(name, FALLBACK_IMAGE_NAME);
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
        let expected = [
            ("--tag", plan.tag.as_str()),
            ("--file", "shell/Dockerfile"),
            ("--build-arg", "BASE=alpine"),
            ("--build-arg", "USER=sally"),
        ];
        for (flag, value) in expected {
            assert!(has(flag, value), "{flag} {value}: {cli_args:?}");
        }
    }

    #[test]
    fn command__any_plan__build_first_the_context_last() {
        let spec = BuildSpec {
            dockerfile: PathBuf::from("shell/Dockerfile"),
            context: PathBuf::from("ctx"),
            args: BTreeMap::new(),
        };
        let plan = BuildPlan::new(&spec, DOCKERFILE);

        let cli_args = plan.command().build_command_args();

        let ends = (cli_args.first().cloned(), cli_args.last().cloned());
        assert_eq!(ends, (Some("build".to_owned()), Some("ctx".to_owned())));
    }
}
