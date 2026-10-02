//! Merge: one layer on top of another, as the fold applies a chain.
//! Invariant: later wins, per field and per key; an entry with
//! `enabled: false` stays in the list, to be dropped only by the fold.
//!
//! Collections are keyed lists: a later entry with an earlier one's key
//! updates it in its place; a new one comes last.

use super::parse::{
    Env, FileItem, HookItem, Hooks, Layer, MountItem, PassthroughItem, Privileges, Share, StateItem,
};

/// An entry of a list that later layers change by its key.
pub trait Keyed: Clone {
    /// The path or name that identifies the entry.
    fn key(&self) -> &str;
    fn key_mut(&mut self) -> &mut String;
    fn enabled(&self) -> bool;
}

/// `over`'s entries on top of `base`'s: one with a key already there updates
/// that entry in its place; a new one comes last.
pub fn merge_keyed<T: Keyed>(base: &[T], over: &[T]) -> Vec<T> {
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

impl Layer {
    /// `over` on top of `self`: its fields where set, its entries per key.
    /// The result is a plain layer: no name, `scan` or `extends`.
    pub fn merge(self, over: &Layer) -> Layer {
        Layer {
            image: over.image.clone().or(self.image),
            state_dir: over.state_dir.clone().or(self.state_dir),
            banner: over.banner.clone().or(self.banner),
            shell: over.shell.clone().or(self.shell),
            persistent: over.persistent.or(self.persistent),
            attach: over.attach.or(self.attach),
            share: self.share.merge(&over.share),
            privileges: self.privileges.merge(&over.privileges),
            env: self.env.merge(&over.env),
            state: merge_keyed(&self.state, &over.state),
            mounts: merge_keyed(&self.mounts, &over.mounts),
            hooks: self.hooks.merge(&over.hooks),
            ..Layer::default()
        }
    }
}

impl Share {
    fn merge(&self, over: &Share) -> Share {
        Share {
            docker: over.docker.or(self.docker),
            host_network: over.host_network.or(self.host_network),
        }
    }
}

impl Privileges {
    fn merge(&self, over: &Privileges) -> Privileges {
        Privileges {
            sudo: over.sudo.or(self.sudo),
        }
    }
}

impl Env {
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

impl Hooks {
    fn merge(&self, over: &Hooks) -> Hooks {
        Hooks {
            create: merge_keyed(&self.create, &over.create),
            attach: merge_keyed(&self.attach, &over.attach),
        }
    }
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

impl Keyed for HookItem {
    fn key(&self) -> &str {
        match self {
            HookItem::Command(command) => command,
            HookItem::Full(spec) => &spec.run,
        }
    }

    fn key_mut(&mut self) -> &mut String {
        match self {
            HookItem::Command(key) => key,
            HookItem::Full(spec) => &mut spec.run,
        }
    }

    fn enabled(&self) -> bool {
        !matches!(self, HookItem::Full(spec) if !spec.enabled)
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

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::path::PathBuf;

    use crate::banner::ART_TEXT;
    use crate::config::effective::{EffectiveEnv, EffectiveHooks, EnvFile, MountEntry};
    use crate::config::parse::MountMode;
    use crate::config::testing::{REPO, effective};
    use crate::constants::DEFAULT_MOUNT_MODE;

    fn mount(path: &str, mode: MountMode) -> MountEntry {
        MountEntry {
            path: path.to_owned(),
            target: None,
            mode,
            file: PathBuf::from(REPO).join("app.vz.yml"),
        }
    }

    const BASE: &str = "\
image: debian
mounts:
  - ~/repos
  - ~/.gitconfig
state:
  - ~/.cache
---
name: writable
extends: default
mounts:
  - ~/repos:rw
---
name: bare
extends: default
mounts:
  - { path: ~/repos, enabled: false }
  - { path: ~/.gitconfig, enabled: false }
state:
  - { path: ~/.cache, enabled: false }
";

    #[test]
    fn merge__nothing_asked_for__the_default_alone() {
        let config = effective(BASE, None);

        let expected = vec![
            mount("~/repos", MountMode::Ro),
            mount("~/.gitconfig", MountMode::Ro),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn merge__entry_with_the_same_path__updates_it_in_place() {
        let config = effective(BASE, Some("writable"));

        let expected = vec![
            mount("~/repos", MountMode::Rw),
            mount("~/.gitconfig", MountMode::Ro),
        ];
        assert_eq!(config.mounts, expected);
    }

    #[test]
    fn merge__entry_disabled__removes_the_earlier_mount() {
        let config = effective(BASE, Some("bare"));

        assert_eq!(config.mounts, []);
    }

    #[test]
    fn merge__entry_disabled__removes_the_earlier_state_path() {
        let config = effective(BASE, Some("bare"));

        assert_eq!(config.state, []);
    }

    /// No layer sets anything beyond the image.
    const IMAGE_ONLY: &str = "image: debian\n";

    #[test]
    fn merge__share_docker__off_unless_a_layer_turns_it_on() {
        let text = "\
image: debian
share:
  docker: true
---
name: offline
extends: default
share: { docker: false }
";
        let cases = [
            ("unset", IMAGE_ONLY, None, false),
            ("turned on", text, None, true),
            ("turned off again", text, Some("offline"), false),
        ];
        for (case, text, config, expected) in cases {
            let docker = effective(text, config).share.docker;

            assert_eq!(docker, expected, "{case}");
        }
    }

    #[test]
    fn merge__share_host_network__off_unless_a_layer_turns_it_on() {
        let text = "\
image: debian
---
name: trusted
extends: default
share: { host_network: true }
---
name: isolated
extends: trusted
share: { host_network: false }
";
        let cases = [
            ("unset", None, false),
            ("turned on", Some("trusted"), true),
            ("turned off again", Some("isolated"), false),
        ];
        for (case, config, expected) in cases {
            let host_network = effective(text, config).share.host_network;

            assert_eq!(host_network, expected, "{case}");
        }
    }

    #[test]
    fn merge__sudo__off_unless_a_layer_grants_it() {
        let text = "\
image: debian
privileges: { sudo: true }
---
name: locked
extends: default
privileges: { sudo: false }
";
        let cases = [
            ("unset", IMAGE_ONLY, None, false),
            ("granted", text, None, true),
            ("taken back", text, Some("locked"), false),
        ];
        for (case, text, config, expected) in cases {
            let sudo = effective(text, config).privileges.sudo;

            assert_eq!(sudo, expected, "{case}");
        }
    }

    #[test]
    fn merge__shell__the_last_layer_to_set_it() {
        let text = "\
image: debian
shell: fish
---
name: plain
extends: default
shell: /bin/sh
---
name: inherits
extends: default
";
        let cases = [
            ("set", text, None, Some("fish")),
            ("set again", text, Some("plain"), Some("/bin/sh")),
            ("inherited", text, Some("inherits"), Some("fish")),
            ("unset", IMAGE_ONLY, None, None),
        ];
        for (case, text, config, expected) in cases {
            let config = effective(text, config);

            assert_eq!(config.shell.as_deref(), expected, "{case}");
        }
    }

    #[test]
    fn merge__state_dir__the_last_layer_to_set_it() {
        let text = "\
image: debian
state_dir: /srv/state
---
name: own
extends: default
state_dir: /srv/own
---
name: inherits
extends: default
";
        let cases = [
            ("set", text, None, Some("/srv/state")),
            ("set again", text, Some("own"), Some("/srv/own")),
            ("inherited", text, Some("inherits"), Some("/srv/state")),
            ("unset", IMAGE_ONLY, None, None),
        ];
        for (case, text, config, expected) in cases {
            let config = effective(text, config);

            assert_eq!(config.state_dir.as_deref(), expected, "{case}");
        }
    }

    #[test]
    fn merge__persistent__off_unless_a_layer_turns_it_on() {
        let text = "\
image: debian
persistent: true
---
name: gone
extends: default
persistent: false
";
        let cases = [
            ("unset", IMAGE_ONLY, None, false),
            ("turned on", text, None, true),
            ("turned off again", text, Some("gone"), false),
        ];
        for (case, text, config, expected) in cases {
            let persistent = effective(text, config).persistent;

            assert_eq!(persistent, expected, "{case}");
        }
    }

    #[test]
    fn merge__attach__off_unless_a_layer_turns_it_on() {
        let text = "\
image: debian
---
name: shared
extends: default
attach: true
---
name: separate
extends: shared
attach: false
";
        let cases = [
            ("unset", None, false),
            ("turned on", Some("shared"), true),
            ("turned off again", Some("separate"), false),
        ];
        for (case, config, expected) in cases {
            let attach = effective(text, config).attach;

            assert_eq!(attach, expected, "{case}");
        }
    }

    #[test]
    fn merge__banner__off_unless_a_layer_turns_it_on_the_later_one_wins() {
        let text = "image: debian\nbanner: true\n---\nname: quiet\nextends: default\n\
                    banner: false\n---\nname: own\nextends: default\nbanner: \"== app ==\"\n";
        let cases = [
            ("turned on", text, None, Some(ART_TEXT)),
            ("turned off again", text, Some("quiet"), None),
            ("art of its own", text, Some("own"), Some("== app ==")),
            ("unset", IMAGE_ONLY, None, None),
        ];
        for (case, text, config, expected) in cases {
            let config = effective(text, config);

            assert_eq!(config.banner.as_deref(), expected, "{case}");
        }
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
---
name: ci
extends: default
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
    fn merge__env_extending__overrides_removes_and_appends() {
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

    const HOOKS: &str = "\
image: debian
hooks:
  create:
    - npm ci
    - { run: \"fish -c 'set -U fish_greeting'\", enabled: false }
    - make setup
  attach:
    - git fetch --quiet
---
name: offline
extends: default
hooks:
  create:
    - { run: npm ci, enabled: false }
    - make setup
    - cargo fetch
  attach:
    - { run: git fetch --quiet, enabled: false }
";

    fn hooks(create: &[&str], attach: &[&str]) -> EffectiveHooks {
        let commands = |list: &[&str]| list.iter().map(|command| command.to_string()).collect();
        EffectiveHooks {
            create: commands(create),
            attach: commands(attach),
        }
    }

    #[test]
    fn merge__hooks__both_forms_in_order_disabled_dropped() {
        let cases = [
            (
                None,
                hooks(&["npm ci", "make setup"], &["git fetch --quiet"]),
            ),
            // npm ci disabled, make setup updated in its place, cargo fetch appended.
            (Some("offline"), hooks(&["make setup", "cargo fetch"], &[])),
        ];
        for (config, expected) in cases {
            let hooks = effective(HOOKS, config).hooks;

            assert_eq!(hooks, expected, "{config:?}");
        }
    }

    const TARGETS: &str = "\
image: debian
mounts:
  - { path: ~/skills, target: ~/.agents/skills }
  - { path: ~/skills, target: ~/.config/opencode/skills }
  - { path: ~/same, target: ~/same }
---
name: agents-only
extends: default
mounts:
  - { path: ~/skills, target: ~/.config/opencode/skills, enabled: false }
";

    #[test]
    fn merge__mount_targets__one_source_at_several_keyed_by_target() {
        let with_target = |target: &str| MountEntry {
            target: Some(target.to_owned()),
            ..mount("~/skills", MountMode::Ro)
        };
        let cases = [
            (
                None,
                vec![
                    with_target("~/.agents/skills"),
                    with_target("~/.config/opencode/skills"),
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
        for (config, expected) in cases {
            let mounts = effective(TARGETS, config).mounts;

            assert_eq!(mounts, expected, "{config:?}");
        }
    }

    /// `~/skills` shown at `~/.agents/skills`, in `mode`.
    fn skills_at_agents(mode: MountMode) -> MountEntry {
        MountEntry {
            target: Some("~/.agents/skills".to_owned()),
            ..mount("~/skills", mode)
        }
    }

    #[test]
    fn merge__mount_string_or_map_form__the_same_entry() {
        let cases = [
            (
                "string, mode",
                "~/repos:rw",
                mount("~/repos", MountMode::Rw),
            ),
            (
                "map, mode",
                "{ path: ~/repos, mode: rw }",
                mount("~/repos", MountMode::Rw),
            ),
            (
                "string, path only",
                "~/repos",
                mount("~/repos", MountMode::Ro),
            ),
            (
                "map, mode ro",
                "{ path: ~/repos, mode: ro }",
                mount("~/repos", MountMode::Ro),
            ),
            (
                "string, target",
                "~/skills:~/.agents/skills",
                skills_at_agents(MountMode::Ro),
            ),
            (
                "map, target",
                "{ path: ~/skills, target: ~/.agents/skills }",
                skills_at_agents(MountMode::Ro),
            ),
            (
                "string, target and mode",
                "~/skills:~/.agents/skills:rw",
                skills_at_agents(MountMode::Rw),
            ),
            (
                "map, target and mode",
                "{ path: ~/skills, target: ~/.agents/skills, mode: rw }",
                skills_at_agents(MountMode::Rw),
            ),
        ];
        for (case, form, expected) in cases {
            let text = format!("image: debian\nmounts:\n  - {form}\n");

            let mounts = effective(&text, None).mounts;

            assert_eq!(mounts, [expected], "{case}");
        }
    }

    #[test]
    fn merge__string_form_extending__layers_by_target_else_path() {
        let cases = [
            (
                "- ~/repos",
                "- ~/repos:rw",
                vec![mount("~/repos", MountMode::Rw)],
            ),
            (
                "- ~/skills:~/.agents/skills",
                "- { path: ~/skills, target: ~/.agents/skills, enabled: false }",
                vec![],
            ),
        ];
        for (base, over, expected) in cases {
            let text = format!(
                "image: debian\nmounts:\n  {base}\n---\nname: p\nextends: default\nmounts:\n  {over}\n"
            );

            let mounts = effective(&text, Some("p")).mounts;

            assert_eq!(mounts, expected, "{base} / {over}");
        }
    }

    #[test]
    fn merge__mount_without_a_mode__has_the_default_mode() {
        let forms = ["~/repos", "{ path: ~/repos }", "~/skills:~/.agents/skills"];
        for form in forms {
            let text = format!("image: debian\nmounts:\n  - {form}\n");

            let modes: Vec<MountMode> = effective(&text, None)
                .mounts
                .iter()
                .map(|entry| entry.mode)
                .collect();

            assert_eq!(modes, vec![DEFAULT_MOUNT_MODE], "{form}");
        }
    }
}
