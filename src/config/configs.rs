//! Step 5 and 6, table and chain: the configurations by scope and name, then
//! the one asked for and those it extends, in the order they apply. Lookups
//! go outward: a repository configuration's `extends` names the
//! repository's configuration, else the library's; a library
//! configuration's, the library's. A configuration never resolves to
//! itself, so a repository's `trusted` with `extends: trusted` means the
//! library's. `default` is the repository's alone. Invariant: one
//! configuration per name per scope; the chain is a straight line, each
//! configuration after its one parent, the one asked for last; unknown
//! names and cycles are refused.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::PathBuf;

use anyhow::{Context, bail};

use super::name::{Config, places, scope_name};
use super::parse::{Banner, ImageSource, Layer};
use super::scan::{Scope, Source};
use crate::constants::{DEFAULT_CONFIG, LIBRARY_BASE};

/// A configuration's place in the table: its scope, then its name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub scope: Scope,
    pub name: String,
}

impl Key {
    pub fn of(config: &Config) -> Self {
        Self {
            scope: config.scope(),
            name: config.name.clone(),
        }
    }
}

/// Every configuration scanned; the repository's first.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub configs: BTreeMap<Key, Config>,
    /// Where the scan looked, for errors.
    pub searched: Vec<PathBuf>,
    /// Every configuration file scanned, in scan order.
    pub files: Vec<Source>,
}

/// The configurations keyed by scope and name. Two of one name in one
/// scope are refused, naming both files.
pub fn by_name(configs: Vec<Config>) -> anyhow::Result<BTreeMap<Key, Config>> {
    let mut table: BTreeMap<Key, Config> = BTreeMap::new();
    for config in configs {
        match table.entry(Key::of(&config)) {
            Entry::Vacant(entry) => {
                entry.insert(config);
            }
            Entry::Occupied(entry) => bail!(
                "two configurations named `{}` in {}: {}",
                config.name,
                scope_name(config.scope()),
                places(&entry.get().source, &config.source)
            ),
        }
    }
    Ok(table)
}

/// A configuration of the chain, and the file that holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub config: String,
    pub file: Source,
}

impl Step {
    pub fn of(config: &Config) -> Self {
        Self {
            config: config.name.clone(),
            file: config.source.clone(),
        }
    }
}

/// The chain of `wanted`, or of `default` without one: its parent's chain,
/// then itself, in fold order.
pub fn resolve_chain<'t>(
    table: &'t Table,
    wanted: Option<&str>,
) -> anyhow::Result<Vec<&'t Config>> {
    let mut config = select(table, wanted)?;
    let mut upward = vec![config];
    while let Some(name) = config.parent() {
        let parent = parent(table, config, name)
            .with_context(|| format!("configuration `{}` extends `{name}`", config.name))?;
        refuse_cycle(&upward, parent)?;
        upward.push(parent);
        config = parent;
    }
    upward.reverse();
    Ok(upward)
}

/// The configuration asked for: the repository's, else the library's. The
/// `default` is the repository's alone: the library holds none.
fn select<'t>(table: &'t Table, wanted: Option<&str>) -> anyhow::Result<&'t Config> {
    match wanted {
        Some(name) => find(table, &[Scope::Repository, Scope::Library], name, None)
            .ok_or_else(|| unknown(table, name)),
        None => match find(table, &[Scope::Repository], DEFAULT_CONFIG, None) {
            Some(config) => Ok(config),
            None => bail!(
                "no default configuration: add vz.yml with `extends: {LIBRARY_BASE}`, or run \
                 with -c NAME"
            ),
        },
    }
}

/// Refuses `parent` when the walk up from the configuration asked for,
/// `upward`, met it already: the configurations from there extend each
/// other.
fn refuse_cycle(upward: &[&Config], parent: &Config) -> anyhow::Result<()> {
    let Some(start) = upward
        .iter()
        .position(|seen| Key::of(seen) == Key::of(parent))
    else {
        return Ok(());
    };
    let names: Vec<&str> = upward[start..]
        .iter()
        .chain([&parent])
        .map(|seen| seen.name.as_str())
        .collect();
    bail!(
        "configurations extend each other in a cycle: {}",
        names.join(" → ")
    )
}

/// The configuration `name` means in `child`'s `extends`: outward from its
/// scope, never itself.
fn parent<'t>(table: &'t Table, child: &Config, name: &str) -> anyhow::Result<&'t Config> {
    let outward: &[Scope] = match child.scope() {
        Scope::Repository => &[Scope::Repository, Scope::Library],
        Scope::Library => &[Scope::Library],
    };
    find(table, outward, name, Some(Key::of(child))).ok_or_else(|| unknown(table, name))
}

/// The first configuration named `name` in `scopes`, other than `except`.
fn find<'t>(
    table: &'t Table,
    scopes: &[Scope],
    name: &str,
    except: Option<Key>,
) -> Option<&'t Config> {
    scopes
        .iter()
        .map(|scope| Key {
            scope: *scope,
            name: name.to_owned(),
        })
        .filter(|key| except.as_ref() != Some(key))
        .find_map(|key| table.configs.get(&key))
}

/// `no configuration `x``, with where the scan looked and the names it found.
fn unknown(table: &Table, name: &str) -> anyhow::Error {
    let searched: Vec<String> = table
        .searched
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let mut found: Vec<&str> = table.configs.keys().map(|key| key.name.as_str()).collect();
    found.sort();
    found.dedup();
    let found = match found.as_slice() {
        [] => "none".to_owned(),
        names => names.join(", "),
    };
    anyhow::anyhow!(
        "no configuration `{name}`; scanned {}; found {found}",
        searched.join(", ")
    )
}

/// A configuration as `vz configs` shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigInfo {
    pub name: String,
    pub file: Source,
    pub extends: Option<String>,
    /// What it changes, in a few words each: `sudo`, `2 mounts`, …
    pub changes: Vec<String>,
}

/// Every configuration, the repository's first, each scope by name.
pub fn list(table: &Table) -> Vec<ConfigInfo> {
    table
        .configs
        .values()
        .map(|config| ConfigInfo {
            name: config.name.clone(),
            file: config.source.clone(),
            extends: config.parent().map(str::to_owned),
            changes: changes(&config.layer),
        })
        .collect()
}

/// What a layer changes, in a few words each.
fn changes(layer: &Layer) -> Vec<String> {
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
    let image = layer.image.as_ref().map(|image| match image {
        ImageSource::Reference(reference) => format!("image {reference}"),
        ImageSource::Build(_) => "its own image".to_owned(),
    });
    let shell = layer.shell.as_ref().map(|shell| format!("shell {shell}"));
    [
        image,
        shell,
        switch(layer.privileges.sudo, "sudo"),
        switch(layer.share.docker, "docker"),
        switch(layer.share.host_network, "host network"),
        count(layer.mounts.len(), "mount"),
        count(layer.state.len(), "state path"),
        count(layer.env.defaults.len(), "env default"),
        count(layer.env.files.len(), "env file"),
        count(layer.env.passthrough.len(), "passthrough"),
        count(layer.hooks.create.len(), "create hook"),
        count(layer.hooks.attach.len(), "attach hook"),
        layer.banner.as_ref().map(|banner| match banner {
            Banner::Switch(true) => "banner".to_owned(),
            Banner::Switch(false) => "no banner".to_owned(),
            Banner::Art(_) => "banner art".to_owned(),
        }),
        switch(layer.persistent, "persistent"),
        switch(layer.attach, "attach"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;
    use crate::config::testing::{message, table};

    /// The chain of `wanted` over `files`, as `scope name` labels.
    fn chain(files: &[(&str, &str)], wanted: Option<&str>) -> anyhow::Result<Vec<String>> {
        let table = table(files)?;
        let chain = resolve_chain(&table, wanted)?;
        Ok(chain
            .iter()
            .map(|config| label(config.scope(), &config.name))
            .collect())
    }

    fn label(scope: Scope, name: &str) -> String {
        match scope {
            Scope::Repository => format!("repository {name}"),
            Scope::Library => format!("library {name}"),
        }
    }

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    const APP: &str = "/home/sally/repos/app/app.vz.yml";
    const CI: &str = "/home/sally/repos/app/ci.vz.yml";
    const LIBRARY_DEFAULT: &str = "/home/sally/.config/viz-shell/default.vz.yml";
    const LIBRARY_TRUSTED: &str = "/home/sally/.config/viz-shell/trusted.vz.yml";
    const LIBRARY_BASE_FILE: &str = "/home/sally/.config/viz-shell/vz-debian-trixie.vz.yml";

    #[test]
    fn by_name__one_name_twice_in_a_scope__refused_naming_both_files() {
        let cases = [
            (
                "in two repository files",
                vec![(APP, "name: ci\n"), (CI, "name: ci\n")],
                "two configurations named `ci` in the repository: \
                 in /home/sally/repos/app/app.vz.yml and in /home/sally/repos/app/ci.vz.yml",
            ),
            (
                "twice in one file",
                vec![(APP, "name: ci\n---\nname: ci\n")],
                "two configurations named `ci` in the repository: \
                 twice in /home/sally/repos/app/app.vz.yml",
            ),
            (
                "a nameless document beside one named default",
                vec![(APP, "image: debian\n---\nname: default\n")],
                "two configurations named `default` in the repository: \
                 twice in /home/sally/repos/app/app.vz.yml",
            ),
            (
                "in the library folder and a folder it lists",
                vec![
                    (LIBRARY_DEFAULT, "scan: [~/team]\n"),
                    (LIBRARY_TRUSTED, "name: trusted\n"),
                    ("/home/sally/team/trusted.vz.yml", "name: trusted\n"),
                ],
                "two configurations named `trusted` in the library: \
                 in /home/sally/.config/viz-shell/trusted.vz.yml \
                 and in /home/sally/team/trusted.vz.yml",
            ),
        ];
        for (case, files, expected) in cases {
            let message = message(table(&files));

            assert!(message.contains(expected), "{case}: {message}");
        }
    }

    #[test]
    fn by_name__one_name_in_each_scope__both_kept() {
        let files = [
            (LIBRARY_DEFAULT, "image: alpine\n"),
            (APP, "image: debian\n"),
        ];

        let keys: Vec<String> = table(&files)
            .unwrap()
            .configs
            .keys()
            .map(|key| label(key.scope, &key.name))
            .collect();

        assert_eq!(keys, labels(&["repository default", "library default"]));
    }

    #[test]
    fn resolve_chain__three_deep__the_parent_s_chain_then_the_configuration() {
        let files = [(
            APP,
            "name: a\nimage: debian\n---\nname: b\nextends: a\n---\nname: c\nextends: b\n",
        )];
        let cases: [(&str, &[&str]); 3] = [
            ("a", &["repository a"]),
            ("b", &["repository a", "repository b"]),
            ("c", &["repository a", "repository b", "repository c"]),
        ];
        for (wanted, expected) in cases {
            let chain = chain(&files, Some(wanted)).unwrap();

            assert_eq!(chain, labels(expected), "{wanted}");
        }
    }

    #[test]
    fn resolve_chain__extends_its_own_name__resolves_outward_to_the_library() {
        let files = [
            (LIBRARY_BASE_FILE, "name: vz-debian-trixie\nimage: debian\n"),
            (
                LIBRARY_TRUSTED,
                "name: trusted\nextends: vz-debian-trixie\nprivileges: { sudo: true }\n",
            ),
            (
                APP,
                "name: vz-debian-trixie\nextends: vz-debian-trixie\n---\n\
                 name: trusted\nextends: trusted\n",
            ),
        ];
        let cases: [(Option<&str>, &[&str]); 2] = [
            (
                Some("vz-debian-trixie"),
                &["library vz-debian-trixie", "repository vz-debian-trixie"],
            ),
            (
                Some("trusted"),
                &[
                    "library vz-debian-trixie",
                    "library trusted",
                    "repository trusted",
                ],
            ),
        ];
        for (wanted, expected) in cases {
            let chain = chain(&files, wanted).unwrap();

            assert_eq!(chain, labels(expected), "{wanted:?}");
        }
    }

    #[test]
    fn resolve_chain__a_name_in_both_scopes__the_repository_s_first() {
        let files = [
            (LIBRARY_DEFAULT, "name: base\nimage: alpine\n"),
            (
                APP,
                "name: base\nimage: debian\n---\nname: ci\nextends: base\n",
            ),
        ];

        let chain = chain(&files, Some("ci")).unwrap();

        assert_eq!(chain, labels(&["repository base", "repository ci"]));
    }

    #[test]
    fn resolve_chain__a_name_only_the_library_has__the_library_s() {
        let files = [
            (LIBRARY_DEFAULT, "name: work\nimage: alpine\n"),
            (CI, "name: ci\nextends: work\n"),
        ];

        let chain = chain(&files, Some("ci")).unwrap();

        assert_eq!(chain, labels(&["library work", "repository ci"]));
    }

    #[test]
    fn resolve_chain__library_configuration_extending_a_repository_name__refused() {
        let files = [
            (LIBRARY_DEFAULT, "name: work\nextends: helper\n"),
            (APP, "name: helper\nimage: debian\n"),
        ];

        let message = message(chain(&files, Some("work")));

        assert!(
            message.contains("configuration `work` extends `helper`: no configuration `helper`"),
            "{message}"
        );
    }

    /// A repository whose `ci` extends a name nobody has.
    const EXTENDS_NOPE: (&str, &str) = (APP, "image: debian\n---\nname: ci\nextends: nope\n");

    #[test]
    fn resolve_chain__unknown_name__refused_naming_it_and_who_asked() {
        let cases = [
            (
                "extended",
                "ci",
                "configuration `ci` extends `nope`: no configuration `nope`",
            ),
            ("asked for with -c", "nope", "no configuration `nope`"),
        ];
        for (case, wanted, expected) in cases {
            let message = message(chain(&[EXTENDS_NOPE], Some(wanted)));

            assert!(message.contains(expected), "{case}: {message}");
        }
    }

    #[test]
    fn resolve_chain__unknown_name__refused_listing_the_folders_scanned_and_names_found() {
        let message = message(chain(&[EXTENDS_NOPE], Some("nope")));

        assert!(
            message.contains(
                "scanned /home/sally/.config/viz-shell, /home/sally/repos/app; found ci, default"
            ),
            "{message}"
        );
    }

    #[test]
    fn resolve_chain__cycle__refused_naming_it() {
        let files = [(
            APP,
            "name: a\nimage: debian\nextends: b\n---\nname: b\nextends: c\n---\nname: c\nextends: b\n",
        )];

        let message = message(chain(&files, Some("a")));

        assert!(
            message.contains("configurations extend each other in a cycle: b → c → b"),
            "{message}"
        );
    }

    #[test]
    fn resolve_chain__library_configuration_extending_its_own_name__refused() {
        let files = [(LIBRARY_DEFAULT, "name: x\nextends: x\n")];

        let message = message(chain(&files, Some("x")));

        assert!(message.contains("no configuration `x`"), "{message}");
    }

    #[test]
    fn resolve_chain__nothing_asked_for__the_repository_s_default() {
        let files = [
            (LIBRARY_DEFAULT, "image: alpine\n"),
            (APP, "image: debian\n"),
        ];

        let chain = chain(&files, None).unwrap();

        assert_eq!(chain, labels(&["repository default"]));
    }

    #[test]
    fn resolve_chain__no_default_in_the_repository__refused_even_with_one_in_the_library() {
        let cases: [(&str, &[(&str, &str)]); 2] = [
            ("none anywhere", &[(CI, "name: ci\nimage: debian\n")]),
            (
                "one in the library",
                &[
                    (LIBRARY_DEFAULT, "image: alpine\n"),
                    (CI, "name: ci\nimage: debian\n"),
                ],
            ),
        ];
        for (case, files) in cases {
            let message = message(chain(files, None));

            assert!(
                message.contains(
                    "no default configuration: add vz.yml with `extends: vz-debian-trixie`, \
                     or run with -c NAME"
                ),
                "{case}: {message}"
            );
        }
    }

    /// A row of `vz configs`: name, file, extends, changes.
    type Row = (String, PathBuf, Option<String>, Vec<String>);

    #[test]
    fn list__both_scopes__the_repository_s_first_with_file_extends_and_changes() {
        let files = [
            (
                LIBRARY_TRUSTED,
                "name: trusted\nprivileges: { sudo: true }\nshare: { docker: true }\n",
            ),
            (
                APP,
                "image: debian\nmounts: [~/a, ~/b]\n---\nname: trusted\nextends: default\n",
            ),
        ];

        let rows: Vec<Row> = list(&table(&files).unwrap())
            .into_iter()
            .map(|info| (info.name, info.file.path, info.extends, info.changes))
            .collect();

        let row = |name: &str, file: &str, extends: Option<&str>, changes: &[&str]| {
            (
                name.to_owned(),
                PathBuf::from(file),
                extends.map(str::to_owned),
                labels(changes),
            )
        };
        let expected = vec![
            row("default", APP, None, &["image debian", "2 mounts"]),
            row("trusted", APP, Some("default"), &[]),
            row("trusted", LIBRARY_TRUSTED, None, &["sudo", "docker"]),
        ];
        assert_eq!(rows, expected);
    }

    #[test]
    fn list__one_key_set__its_change_in_a_few_words() {
        let cases = [
            ("a reference", "image: debian\n", "image debian"),
            (
                "a build",
                "image: { dockerfile: Dockerfile }\n",
                "its own image",
            ),
            ("a shell", "shell: zsh\n", "shell zsh"),
            ("a switch on", "privileges: { sudo: true }\n", "sudo"),
            ("a switch off", "privileges: { sudo: false }\n", "no sudo"),
            ("one entry", "mounts: [~/a]\n", "1 mount"),
            ("two entries", "mounts: [~/a, ~/b]\n", "2 mounts"),
            ("the banner off", "banner: false\n", "no banner"),
            ("banner art", "banner: \"hi\"\n", "banner art"),
        ];
        for (case, text, expected) in cases {
            let table = table(&[(APP, text)]).unwrap();

            let changes = list(&table).remove(0).changes;

            assert_eq!(changes, [expected], "{case}");
        }
    }
}
