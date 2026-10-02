//! Step 4, name: each document becomes a [`Config`], named by its `name:`
//! key. Invariant: in each scope, at most one configuration goes without a
//! name, and it is `default`; two nameless ones are refused, naming both
//! files. A configuration named `default` is an ordinary one.

use anyhow::bail;

use super::parse::Layer;
use super::scan::{Scope, Source};
use crate::constants::DEFAULT_CONFIG;

/// A document of a file, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub source: Source,
    pub layer: Layer,
}

/// A configuration: a document, named.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub source: Source,
    pub layer: Layer,
}

impl Config {
    pub fn scope(&self) -> Scope {
        self.source.scope
    }

    /// The configuration it starts from.
    pub fn parent(&self) -> Option<&str> {
        self.layer.extends.as_deref()
    }
}

/// Every document named: its `name:`, else, as its scope's one nameless
/// configuration, `default`.
pub fn name(documents: Vec<Document>) -> anyhow::Result<Vec<Config>> {
    for scope in [Scope::Repository, Scope::Library] {
        let nameless: Vec<&Source> = documents
            .iter()
            .filter(|document| document.source.scope == scope && document.layer.name.is_none())
            .map(|document| &document.source)
            .collect();
        if let [first, second, ..] = nameless.as_slice() {
            bail!(
                "two configurations without `name:` in {}: {}; one may go without, and is `{DEFAULT_CONFIG}`",
                scope_name(scope),
                places(first, second)
            );
        }
    }
    Ok(documents
        .into_iter()
        .map(|document| Config {
            name: document
                .layer
                .name
                .clone()
                .unwrap_or_else(|| DEFAULT_CONFIG.to_owned()),
            source: document.source,
            layer: document.layer,
        })
        .collect())
}

/// `the repository`, `the library`.
pub fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Repository => "the repository",
        Scope::Library => "the library",
    }
}

/// Where two documents are: `in A and in B`, or `twice in A`.
pub fn places(first: &Source, second: &Source) -> String {
    match first.path == second.path {
        true => format!("twice in {}", first.path.display()),
        false => format!(
            "in {} and in {}",
            first.path.display(),
            second.path.display()
        ),
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::testing::message;

    /// A document of `path`, named `name` or nameless, in `scope`.
    fn document(path: &str, scope: Scope, name: Option<&str>) -> Document {
        Document {
            source: Source {
                path: PathBuf::from(path),
                scope,
            },
            layer: Layer {
                name: name.map(str::to_owned),
                ..Layer::default()
            },
        }
    }

    const APP: &str = "/home/sally/repos/app/app.vz.yml";
    const CI: &str = "/home/sally/repos/app/ci.vz.yml";
    const LIBRARY: &str = "/home/sally/.config/viz-shell/default.vz.yml";

    #[test]
    fn name__documents__their_name_else_default_for_the_one_nameless_per_scope() {
        use Scope::{Library, Repository};
        let cases: [(&str, Vec<Document>, &[&str]); 5] = [
            (
                "named",
                vec![document(APP, Repository, Some("app"))],
                &["app"],
            ),
            (
                "nameless",
                vec![document(APP, Repository, None)],
                &["default"],
            ),
            (
                "one nameless beside named ones",
                vec![
                    document(APP, Repository, None),
                    document(CI, Repository, Some("ci")),
                ],
                &["default", "ci"],
            ),
            (
                "one nameless in each scope",
                vec![
                    document(APP, Repository, None),
                    document(LIBRARY, Library, None),
                ],
                &["default", "default"],
            ),
            (
                "named default, an ordinary name",
                vec![document(APP, Repository, Some("default"))],
                &["default"],
            ),
        ];
        for (case, documents, expected) in cases {
            let configs = name(documents).unwrap();

            let names: Vec<&str> = configs.iter().map(|config| config.name.as_str()).collect();
            assert_eq!(names, expected, "{case}");
        }
    }

    #[test]
    fn name__two_nameless_in_one_scope__refused_naming_both_files() {
        use Scope::Repository;
        let cases = [
            (
                "in two files",
                vec![
                    document(APP, Repository, None),
                    document(CI, Repository, None),
                ],
                "two configurations without `name:` in the repository: \
                 in /home/sally/repos/app/app.vz.yml and in /home/sally/repos/app/ci.vz.yml",
            ),
            (
                "twice in one file",
                vec![
                    document(APP, Repository, None),
                    document(APP, Repository, None),
                ],
                "two configurations without `name:` in the repository: \
                 twice in /home/sally/repos/app/app.vz.yml",
            ),
        ];
        for (case, documents, expected) in cases {
            let message = message(name(documents));

            assert!(message.contains(expected), "{case}: {message}");
        }
    }
}
