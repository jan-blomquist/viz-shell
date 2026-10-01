//! The library a first run writes: one template, `vz-debian-trixie`, its
//! configuration and the Dockerfile it builds, embedded at build time from
//! `templates/`. Invariant: a first run is a library with no configuration
//! named `vz-debian-trixie`, by its documents, not its filenames; a file
//! present is never overwritten.

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::Files;
use super::parse::parse;
use super::scan::is_config_file;
use crate::constants::{LIBRARY_BASE, LIBRARY_BASE_DOCKERFILE, LIBRARY_BASE_FILE};

/// The library's base configuration: `templates/vz-debian-trixie.vz.yml`.
pub const BASE_TEMPLATE: &str = include_str!("../../templates/vz-debian-trixie.vz.yml");

/// The base image it builds: `templates/vz-debian-trixie.Dockerfile`.
pub const BASE_DOCKERFILE_TEMPLATE: &str =
    include_str!("../../templates/vz-debian-trixie.Dockerfile");

/// The templates by the name each is written under.
pub const TEMPLATES: [(&str, &str); 2] = [
    (LIBRARY_BASE_FILE, BASE_TEMPLATE),
    (LIBRARY_BASE_DOCKERFILE, BASE_DOCKERFILE_TEMPLATE),
];

/// A first run: no configuration file of the library folder holds a
/// configuration named `vz-debian-trixie`.
pub fn needs_scaffold(library: &Path, files: &impl Files) -> bool {
    let Ok(file_names) = files.list(library) else {
        return false;
    };
    !file_names
        .iter()
        .filter(|file_name| is_config_file(file_name))
        .any(|file_name| holds_base(&library.join(file_name), files))
}

/// Whether a file holds `vz-debian-trixie`; one that cannot be read or
/// parsed is taken to, so a broken library is reported by loading, not
/// written over.
fn holds_base(path: &Path, files: &impl Files) -> bool {
    let Some(documents) = files.read(path).ok().and_then(|text| parse(&text).ok()) else {
        return true;
    };
    documents
        .iter()
        .any(|document| document.name.as_deref() == Some(LIBRARY_BASE))
}

/// Writes each template missing from `dir`; returns the paths written.
pub fn scaffold(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut written = Vec::new();
    for (file_name, content) in TEMPLATES {
        let path = dir.join(file_name);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;
    use crate::config::parse::{ImageSource, parse};
    use crate::config::testing::{FakeFiles, LIBRARY, effective};
    use crate::config::{RealFiles, Request, load};

    /// Files by path, each with its text.
    type LibraryFiles<'a> = &'a [(&'a str, &'a str)];

    #[test]
    fn needs_scaffold__no_file_holds_the_base__a_first_run() {
        let cases: [(&str, LibraryFiles); 5] = [
            ("empty", &[]),
            (
                "only another configuration",
                &[("/home/sally/.config/viz-shell/work.vz.yml", "name: work\n")],
            ),
            (
                "the template's filename, named otherwise",
                &[(
                    "/home/sally/.config/viz-shell/vz-debian-trixie.vz.yml",
                    "name: mine\n",
                )],
            ),
            (
                "nameless: a default, not the base",
                &[(
                    "/home/sally/.config/viz-shell/mine.vz.yml",
                    "image: debian\n",
                )],
            ),
            (
                "not a configuration file",
                &[(
                    "/home/sally/.config/viz-shell/base.yml",
                    "name: vz-debian-trixie\n",
                )],
            ),
        ];
        for (case, files) in cases {
            let first_run = needs_scaffold(Path::new(LIBRARY), &FakeFiles::with(files));

            assert!(first_run, "{case}");
        }
    }

    #[test]
    fn needs_scaffold__a_file_holds_the_base_or_is_broken__not_a_first_run() {
        let cases: [(&str, LibraryFiles); 3] = [
            (
                "named vz-debian-trixie, any filename",
                &[(
                    "/home/sally/.config/viz-shell/mine.vz.yml",
                    "name: vz-debian-trixie\n",
                )],
            ),
            (
                "in a later document",
                &[(
                    "/home/sally/.config/viz-shell/mine.vz.yml",
                    "name: a\n---\nname: vz-debian-trixie\n",
                )],
            ),
            (
                "broken",
                &[("/home/sally/.config/viz-shell/mine.vz.yml", "imgae: x\n")],
            ),
        ];
        for (case, files) in cases {
            let first_run = needs_scaffold(Path::new(LIBRARY), &FakeFiles::with(files));

            assert!(!first_run, "{case}");
        }
    }

    #[test]
    fn scaffold__empty_folder__writes_both_templates() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".config/viz-shell");

        let written = scaffold(&dir).unwrap();

        let contents: Vec<String> = written
            .iter()
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect();
        assert_eq!(contents, [BASE_TEMPLATE, BASE_DOCKERFILE_TEMPLATE]);
    }

    /// A library folder holding its own base Dockerfile.
    fn library_with_own_dockerfile() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LIBRARY_BASE_DOCKERFILE), "FROM mine\n").unwrap();
        dir
    }

    #[test]
    fn scaffold__file_present__kept_as_it_is() {
        let dir = library_with_own_dockerfile();

        scaffold(dir.path()).unwrap();

        let dockerfile = std::fs::read_to_string(dir.path().join(LIBRARY_BASE_DOCKERFILE)).unwrap();
        assert_eq!(dockerfile, "FROM mine\n");
    }

    #[test]
    fn scaffold__file_present__only_the_missing_one_written() {
        let dir = library_with_own_dockerfile();

        let written = scaffold(dir.path()).unwrap();

        assert_eq!(written, [dir.path().join(LIBRARY_BASE_FILE)]);
    }

    #[test]
    fn base_template__by_name__builds_the_dockerfile_beside_it() {
        let home = tempfile::tempdir().unwrap();
        let library = home.path().join(".config/viz-shell");
        let repo_root = home.path().join("repos/app");
        std::fs::create_dir_all(&repo_root).unwrap();
        scaffold(&library).unwrap();
        let request = Request {
            home: home.path().to_owned(),
            repo_root,
            library_dir: library.clone(),
            extra_files: vec![],
            config: Some(LIBRARY_BASE.to_owned()),
        };

        let loaded = load(&request, &RealFiles).unwrap();

        let ImageSource::Build(spec) = loaded.effective.image().clone() else {
            panic!("the library's base builds its image");
        };
        assert_eq!(spec.dockerfile, library.join(LIBRARY_BASE_DOCKERFILE));
    }

    #[test]
    fn base_template__every_option_at_its_default__the_same_as_without_them() {
        let minimal = "\
name: vz-debian-trixie
image: { dockerfile: vz-debian-trixie.Dockerfile }
banner: true
env: { files: [environment], passthrough: [EDITOR] }
";

        let explicit = effective(BASE_TEMPLATE, Some(LIBRARY_BASE));
        let without = effective(minimal, Some(LIBRARY_BASE));

        assert_eq!(explicit, without);
    }

    /// A round trip: the expectation is the first rendering, on purpose.
    #[test]
    fn base_template__to_yaml__parses_back_to_the_same_yaml() {
        let yaml = effective(BASE_TEMPLATE, Some(LIBRARY_BASE))
            .to_yaml()
            .unwrap();

        assert_eq!(effective(&yaml, None).to_yaml().unwrap(), yaml);
    }

    #[test]
    fn base_template__named_vz_debian_trixie__extending_nothing() {
        let layer = parse(BASE_TEMPLATE).unwrap().remove(0);

        assert_eq!(
            (layer.name.as_deref(), layer.extends),
            (Some(LIBRARY_BASE), None)
        );
    }
}
