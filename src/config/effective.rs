//! Step 7, fold: the chain's configurations, in order, into the [`Effective`]
//! configuration vz runs with. Invariant: later wins per field and per key;
//! removed entries are gone, shorthands spelled out; every `image:` of the
//! chain is kept, bottom first, for the build plan to stack or replace; each
//! mount names the file that set it last.

use std::path::PathBuf;

use anyhow::{Context, ensure};

use super::merge::Keyed;
use super::name::Config;
use super::parse::{
    Banner, Env, EnvScalar, FileItem, FileSpec, HookItem, Hooks, ImageSource, Layer, MountItem,
    MountMode, MountSpec, PassthroughItem, Privileges, Share, StateItem, StateKind, StateSpec,
};
use crate::banner::ART_TEXT;

/// The configuration vz runs with.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    /// Every `image:` of the chain, bottom first; one set again right above
    /// itself counts once. Never empty.
    pub images: Vec<ImageLayer>,
    pub state_dir: Option<String>,
    /// The art above an interactive shell; none without a banner.
    pub banner: Option<String>,
    pub shell: Option<String>,
    pub persistent: bool,
    pub attach: bool,
    pub share: Shared,
    pub privileges: Granted,
    pub env: EffectiveEnv,
    pub state: Vec<StateEntry>,
    pub mounts: Vec<MountEntry>,
    pub hooks: EffectiveHooks,
}

/// An `image:` of the chain, with the configuration and the file that set it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageLayer {
    pub config: String,
    pub file: PathBuf,
    pub source: ImageSource,
}

/// The hooks' commands, in order, without removed ones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectiveHooks {
    pub create: Vec<String>,
    pub attach: Vec<String>,
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
    /// Absolute, as resolved from the file that set it.
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
    /// The configuration file that set it last.
    pub file: PathBuf,
}

/// The chain's configurations folded, bottom first. Refused when none sets
/// an image.
pub fn fold(chain: &[&Config]) -> anyhow::Result<Effective> {
    let merged = chain
        .iter()
        .fold(Layer::default(), |base, config| base.merge(&config.layer));
    let top = chain.last().map_or("", |config| config.name.as_str());
    ensure!(
        merged.image.is_some(),
        "no image: neither `{top}` nor a configuration it extends sets `image:`"
    );
    let mut images: Vec<ImageLayer> = chain
        .iter()
        .filter_map(|config| {
            Some(ImageLayer {
                config: config.name.clone(),
                file: config.source.path.clone(),
                source: config.layer.image.clone()?,
            })
        })
        .collect();
    images.dedup_by(|above, below| above.source == below.source);
    let mount_file = |key: &str| {
        chain
            .iter()
            .rev()
            .find(|config| config.layer.mounts.iter().any(|entry| entry.key() == key))
            .map(|config| config.source.path.clone())
            .expect("an effective mount comes from a configuration")
    };
    Ok(Effective::resolve(merged, images, mount_file))
}

impl Effective {
    /// `mount_file` names the file that set a mount last, by its key.
    fn resolve(
        layer: Layer,
        images: Vec<ImageLayer>,
        mount_file: impl Fn(&str) -> PathBuf,
    ) -> Self {
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
                    mode: MountMode::default(),
                    file: mount_file(entry.key()),
                },
                MountItem::Full(spec) => MountEntry {
                    path: spec.path.clone(),
                    target: spec.target.clone().filter(|target| *target != spec.path),
                    mode: spec.mode,
                    file: mount_file(entry.key()),
                },
            })
            .collect();
        Self {
            images,
            state_dir: layer.state_dir,
            banner: art(layer.banner),
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
            hooks: EffectiveHooks::resolve(&layer.hooks),
        }
    }

    /// The top of the image chain: the image of the last section that set one.
    pub fn image(&self) -> &ImageSource {
        &self
            .images
            .last()
            .expect("an effective configuration has an image")
            .source
    }

    /// The image chain, bottom first, without where each was set.
    pub fn image_sources(&self) -> Vec<ImageSource> {
        self.images
            .iter()
            .map(|image| image.source.clone())
            .collect()
    }

    /// As a configuration file: every entry in its shortest
    /// form that says the same, so it parses back to the same configuration.
    /// Only the top image: `--show-effective-config` lists the chain above.
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
            .map(|entry| {
                MountItem::from(MountSpec {
                    path: entry.path.clone(),
                    target: entry.target.clone(),
                    mode: entry.mode,
                    enabled: true,
                })
            })
            .collect();
        Layer {
            image: Some(self.image().clone()),
            state_dir: self.state_dir.clone(),
            banner: Some(match &self.banner {
                None => Banner::Switch(false),
                Some(art) if art == ART_TEXT => Banner::Switch(true),
                Some(art) => Banner::Art(art.clone()),
            }),
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
            hooks: self.hooks.to_hooks(),
            ..Layer::default()
        }
    }
}

/// The art a `banner:` shows: none for `false` or no banner, the built-in
/// for `true`, else its own, each line's trailing whitespace and the final
/// newlines trimmed.
fn art(banner: Option<Banner>) -> Option<String> {
    match banner? {
        Banner::Switch(false) => None,
        Banner::Switch(true) => Some(ART_TEXT.to_owned()),
        Banner::Art(text) => {
            let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
            Some(lines.join("\n").trim_end_matches('\n').to_owned())
        }
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

impl EffectiveHooks {
    fn resolve(hooks: &Hooks) -> Self {
        let commands = |items: &[HookItem]| {
            items
                .iter()
                .filter(|entry| entry.enabled())
                .map(|entry| entry.key().to_owned())
                .collect()
        };
        Self {
            create: commands(&hooks.create),
            attach: commands(&hooks.attach),
        }
    }

    fn to_hooks(&self) -> Hooks {
        let items = |commands: &[String]| commands.iter().cloned().map(HookItem::Command).collect();
        Hooks {
            create: items(&self.create),
            attach: items(&self.attach),
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::parse::BuildSpec;
    use crate::config::testing::{effective, error, load, message};
    use crate::constants::DEFAULT_MOUNT_MODE;

    fn state(path: &str, kind: StateKind, init: Option<&str>) -> StateEntry {
        StateEntry {
            path: path.to_owned(),
            kind,
            init: init.map(str::to_owned),
        }
    }

    fn mount(path: &str, mode: MountMode, file: &str) -> MountEntry {
        MountEntry {
            path: path.to_owned(),
            target: None,
            mode,
            file: PathBuf::from(file),
        }
    }

    const APP: &str = "/home/sally/repos/app/app.vz.yml";
    const BASE_FILE: &str = "/home/sally/.config/viz-shell/base.vz.yml";

    /// Three steps: the library's `base`, the repository's `default`
    /// extending it, and the repository's `extra` extending that.
    const THREE: [(&str, &str); 2] = [
        (
            BASE_FILE,
            "name: base\nimage: debian\nshell: bash\nbanner: true\nmounts: [~/shared, ~/base]\n\
             env: { defaults: { WHO: base, BASE: \"yes\" } }\n",
        ),
        (
            APP,
            "extends: base\nshell: zsh\nmounts: [~/shared:rw]\nenv: { defaults: { WHO: repo } }\n\
             ---\nname: extra\nextends: default\nenv: { defaults: { WHO: extra } }\nmounts: [~/extra]\n",
        ),
    ];

    /// `extra` of [`THREE`], folded.
    fn three_steps() -> Effective {
        load(&THREE, Some("extra")).unwrap().effective
    }

    #[test]
    fn fold__three_steps__the_last_shell_set() {
        let config = three_steps();

        assert_eq!(config.shell.as_deref(), Some("zsh"));
    }

    #[test]
    fn fold__three_steps__a_banner_set_below_kept() {
        let config = three_steps();

        assert_eq!(config.banner.as_deref(), Some(ART_TEXT));
    }

    #[test]
    fn fold__three_steps__later_wins_each_env_default() {
        let config = three_steps();

        let expected = [
            ("BASE".to_owned(), "yes".to_owned()),
            ("WHO".to_owned(), "extra".to_owned()),
        ];
        assert_eq!(config.env.defaults, expected);
    }

    #[test]
    fn fold__three_steps__later_wins_each_mount_from_the_file_that_set_it_last() {
        let config = three_steps();

        let expected = [
            mount("/home/sally/shared", MountMode::Rw, APP),
            mount("/home/sally/base", DEFAULT_MOUNT_MODE, BASE_FILE),
            mount("/home/sally/extra", DEFAULT_MOUNT_MODE, APP),
        ];
        assert_eq!(config.mounts, expected);
    }

    fn build(path: &str) -> ImageSource {
        let dockerfile = PathBuf::from(path);
        ImageSource::Build(BuildSpec {
            context: dockerfile.parent().unwrap().to_owned(),
            dockerfile,
            args: BTreeMap::new(),
        })
    }

    fn image(config: &str, file: &str, source: ImageSource) -> ImageLayer {
        ImageLayer {
            config: config.to_owned(),
            file: PathBuf::from(file),
            source,
        }
    }

    #[test]
    fn fold__images__every_step_s_bottom_first_with_its_configuration_and_file() {
        let files = [
            (
                "/home/sally/.config/viz-shell/default.vz.yml",
                "image: { dockerfile: base.Dockerfile }\n",
            ),
            (
                APP,
                "extends: default\nimage: { dockerfile: Dockerfile }\n\
                 ---\nname: pulled\nextends: default\nimage: debian\n",
            ),
        ];

        let images = load(&files, Some("pulled")).unwrap().effective.images;

        let expected = vec![
            image(
                "default",
                "/home/sally/.config/viz-shell/default.vz.yml",
                build("/home/sally/.config/viz-shell/base.Dockerfile"),
            ),
            image("default", APP, build("/home/sally/repos/app/Dockerfile")),
            image("pulled", APP, ImageSource::Reference("debian".to_owned())),
        ];
        assert_eq!(images, expected);
    }

    #[test]
    fn fold__same_image_set_again__counts_once() {
        let config = effective(
            "image: debian\n---\nname: p\nextends: default\nimage: debian\n",
            Some("p"),
        );

        let sources = config.image_sources();

        assert_eq!(sources, [ImageSource::Reference("debian".to_owned())]);
    }

    #[test]
    fn fold__no_image_in_the_chain__refused_naming_the_configuration() {
        let files = [
            (
                "/home/sally/.config/viz-shell/trusted.vz.yml",
                "name: trusted\nprivileges: { sudo: true }\n",
            ),
            (APP, "image: debian\n"),
        ];

        let message = message(load(&files, Some("trusted")));

        assert!(
            message.contains(
                "no image: neither `trusted` nor a configuration it extends sets `image:`"
            ),
            "{message}"
        );
    }

    #[test]
    fn fold__no_image_in_the_default__refused() {
        let message = error("mounts: [~/repos]\n");

        assert!(
            message.contains("no image: neither `default` nor a configuration it extends"),
            "{message}"
        );
    }

    #[test]
    fn fold__image_reference__reads_reference() {
        let config = effective("image: hello-world\n", None);

        assert_eq!(
            *config.image(),
            ImageSource::Reference("hello-world".to_owned())
        );
    }

    #[test]
    fn fold__dockerfile_only__defaults_context_and_args() {
        let config = effective("image:\n  dockerfile: Dockerfile\n", None);

        let expected = BuildSpec {
            dockerfile: PathBuf::from("Dockerfile"),
            context: PathBuf::from("."),
            args: BTreeMap::new(),
        };
        assert_eq!(*config.image(), ImageSource::Build(expected));
    }

    #[test]
    fn fold__state_forms__spelled_out_in_order_disabled_dropped() {
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
    fn fold__mount_forms__bare_is_read_only_disabled_dropped() {
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
            mount("~/a", MountMode::Ro, APP),
            mount("~/b", MountMode::Ro, APP),
            mount("~/c", MountMode::Rw, APP),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn fold__env__values_as_text_files_in_order() {
        let text = "\
image: debian
env:
  defaults: { RUST_LOG: info, PORT: 8080, DEBUG: false }
  files: [.env, { path: .env.old, required: true }, { path: .gone, enabled: false }]
  passthrough: [GH_TOKEN, \"FMP_*\"]
";

        let env = effective(text, None).env;

        let expected = EffectiveEnv {
            defaults: vec![
                ("DEBUG".to_owned(), "false".to_owned()),
                ("PORT".to_owned(), "8080".to_owned()),
                ("RUST_LOG".to_owned(), "info".to_owned()),
            ],
            files: vec![
                EnvFile {
                    path: ".env".to_owned(),
                    required: false,
                },
                EnvFile {
                    path: ".env.old".to_owned(),
                    required: true,
                },
            ],
            passthrough: vec!["GH_TOKEN".to_owned(), "FMP_*".to_owned()],
        };
        assert_eq!(env, expected);
    }

    #[test]
    fn to_yaml__effective_configurations__parse_back_to_themselves() {
        let cases = [
            (
                "image: debian\nshare: { docker: true }\nmounts: [~/a, ~/b:rw]\nstate: [~/.c]\n",
                None,
            ),
            (
                "image: debian\nenv:\n  defaults: { A: x }\n  files: [{ path: .env, required: true }, .env.b]\n  \
                 passthrough: [GH_TOKEN]\n---\nname: ci\nextends: default\nenv: { defaults: { A: null } }\n",
                Some("ci"),
            ),
            (
                "image: debian\nhooks: { create: [npm ci], attach: [ls, { run: pwd, enabled: false }] }\n",
                None,
            ),
            (
                "image: debian\nmounts:\n  - { path: ~/skills, target: ~/.agents/skills }\n  \
                 - { path: ~/same, target: ~/same }\n",
                None,
            ),
        ];
        // A round trip: the expectation is the configuration written, on purpose.
        for (text, config) in cases {
            let config = effective(text, config);

            let yaml = config.to_yaml().unwrap();

            assert_eq!(effective(&yaml, None), config, "{yaml}");
        }
    }

    #[test]
    fn fold__banner__the_art_it_shows() {
        let cases = [
            ("unset", "image: debian\n", None),
            ("off", "image: debian\nbanner: false\n", None),
            ("on", "image: debian\nbanner: true\n", Some(ART_TEXT)),
            (
                "art of its own",
                "image: debian\nbanner: \"== app ==\"\n",
                Some("== app =="),
            ),
            ("empty art", "image: debian\nbanner: \"\"\n", Some("")),
            (
                "trailing whitespace and newlines trimmed",
                "image: debian\nbanner: |\n  == app ==  \n  v1\n\n",
                Some("== app ==\nv1"),
            ),
        ];
        for (case, text, expected) in cases {
            let config = effective(text, None);

            assert_eq!(config.banner.as_deref(), expected, "{case}");
        }
    }

    /// Banners of each kind: none, the built-in art, art of its own.
    const BANNERS: [(&str, &str); 3] = [
        ("none", "image: debian\n"),
        ("the built-in art", "image: debian\nbanner: true\n"),
        (
            "art of its own",
            "image: debian\nbanner: |\n  == app ==\n    v1\n",
        ),
    ];

    #[test]
    fn to_yaml__banner__false_true_or_the_art() {
        let expected = [
            "banner: false\n",
            "banner: true\n",
            "banner: |-\n  == app ==\n    v1\n",
        ];
        for ((case, text), expected) in BANNERS.into_iter().zip(expected) {
            let yaml = effective(text, None).to_yaml().unwrap();

            assert!(yaml.contains(expected), "{case}: {yaml}");
        }
    }

    /// A round trip: the expectation is the banner written, on purpose.
    #[test]
    fn to_yaml__banner__parses_back_to_itself() {
        for (case, text) in BANNERS {
            let config = effective(text, None);

            let yaml = config.to_yaml().unwrap();

            assert_eq!(effective(&yaml, None).banner, config.banner, "{case}");
        }
    }

    #[test]
    fn to_yaml__hooks__bare_commands() {
        let config = effective(
            "image: debian\nhooks: { create: [{ run: npm ci }] }\n",
            None,
        );

        let yaml = config.to_yaml().unwrap();

        assert!(yaml.contains("hooks:\n  create:\n  - npm ci\n"), "{yaml}");
    }

    #[test]
    fn to_yaml__no_hooks__no_hooks_key() {
        let all_disabled = "\
image: debian
hooks:
  attach: [ls]
---
name: quiet
extends: default
hooks:
  attach: [{ run: ls, enabled: false }]
";
        let cases = [
            ("none set", "image: debian\n", None),
            ("every one disabled", all_disabled, Some("quiet")),
        ];
        for (case, text, config) in cases {
            let yaml = effective(text, config).to_yaml().unwrap();

            assert!(!yaml.contains("hooks"), "{case}: {yaml}");
        }
    }

    /// The mode a mount must name, whichever the default is: the one place
    /// a test computes, so that flipping DEFAULT_MOUNT_MODE needs no test edit.
    fn non_default_mode() -> &'static str {
        match DEFAULT_MOUNT_MODE {
            MountMode::Ro => "rw",
            MountMode::Rw => "ro",
        }
    }

    #[test]
    fn to_yaml__mount_in_the_default_mode__bare() {
        let config = effective("image: debian\nmounts: [~/repos]\n", None);

        let yaml = config.to_yaml().unwrap();

        assert!(yaml.contains("mounts:\n- ~/repos\n"), "{yaml}");
    }

    #[test]
    fn to_yaml__mount_in_the_other_mode__the_map_form_naming_it() {
        let word = non_default_mode();
        let config = effective(
            &format!("image: debian\nmounts: [\"~/notes:{word}\"]\n"),
            None,
        );

        let yaml = config.to_yaml().unwrap();

        assert!(
            yaml.contains(&format!("- path: ~/notes\n  mode: {word}\n")),
            "{yaml}"
        );
    }

    #[test]
    fn to_yaml__mount_with_a_target__the_map_form() {
        let config = effective(
            "image: debian\nmounts: [\"~/skills:~/.agents/skills\"]\n",
            None,
        );

        let yaml = config.to_yaml().unwrap();

        assert!(
            yaml.contains("- path: ~/skills\n  target: ~/.agents/skills\n"),
            "{yaml}"
        );
    }

    #[test]
    fn to_yaml__state_dir_in_map_form__bare() {
        let config = effective(
            "image: debian\nstate:\n  - { path: ~/.a, type: dir }\n",
            None,
        );

        let yaml = config.to_yaml().unwrap();

        assert!(yaml.contains("state:\n- ~/.a\n"), "{yaml}");
    }

    #[test]
    fn to_yaml__mounts_in_both_modes__only_one_names_its_mode() {
        let text = "\
image: debian
mounts:
  - { path: ~/b, mode: ro }
  - { path: ~/c, mode: rw }
";
        let config = effective(text, None);

        let yaml = config.to_yaml().unwrap();

        assert_eq!(yaml.matches("mode:").count(), 1, "{yaml}");
    }

    #[test]
    fn to_yaml__image_chain__the_lower_image_not_written() {
        let text =
            "image: debian\n---\nname: p\nextends: default\nimage: { dockerfile: Dockerfile }\n";
        let config = effective(text, Some("p"));

        let yaml = config.to_yaml().unwrap();

        assert!(!yaml.contains("debian"), "{yaml}");
    }
}
