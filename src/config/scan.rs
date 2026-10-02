//! Step 1, scan: the configuration files and their text, in scan order: the
//! library folder, the folders its files list under `scan:`, the repository
//! root, then the `-f` files. Invariant: in a folder, only `*.vz.yml` and
//! `*.vz.yaml` are read, and their names mean nothing more; an `-f` file is
//! read whatever its name; each file once; `scan:` is followed only from the
//! library folder's files.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;

use super::Files;
use super::paths;
use crate::constants::{CONFIG_FILE_SUFFIXES, SHORT_CONFIG_FILE};

/// Whose configurations a file holds. Lookups go outward: a repository
/// configuration sees the repository's, then the library's; a library
/// configuration sees the library's. The repository's sort first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Repository,
    Library,
}

/// A configuration file.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub path: PathBuf,
    pub scope: Scope,
}

impl Source {
    /// Its folder: relative paths in it are relative to this.
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("/"))
    }
}

/// A file found, with its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Scanned {
    pub source: Source,
    pub text: String,
}

/// What a scan found, and where it looked.
#[derive(Debug, Clone, PartialEq)]
pub struct Scan {
    /// The folders, then the `-f` files, in scan order.
    pub searched: Vec<PathBuf>,
    pub files: Vec<Scanned>,
}

/// Where to look.
#[derive(Debug, Clone, Copy)]
pub struct Folders<'a> {
    pub library: &'a Path,
    pub repo_root: &'a Path,
    /// `-f` files: repository files wherever they are.
    pub extra_files: &'a [PathBuf],
    /// For `~` and `${home}` in `scan:`.
    pub home: &'a Path,
}

/// Every configuration file, in scan order.
pub fn collect(folders: &Folders, files: &impl Files) -> anyhow::Result<Scan> {
    let library = read_folder(folders.library, Scope::Library, files)?;
    let extra_dirs = listed_folders(&library, folders)?;
    let mut elsewhere = Vec::new();
    for dir in &extra_dirs {
        elsewhere.extend(read_folder(dir, Scope::Library, files)?);
    }
    elsewhere.extend(read_folder(folders.repo_root, Scope::Repository, files)?);
    for path in folders.extra_files {
        let already = elsewhere.iter().any(|file| file.source.path == *path);
        if !already {
            elsewhere.push(read(source(path.to_owned(), Scope::Repository), files)?);
        }
    }
    refuse_scan(&elsewhere, folders.library)?;
    let searched = std::iter::once(folders.library.to_owned())
        .chain(extra_dirs)
        .chain([folders.repo_root.to_owned()])
        .chain(folders.extra_files.iter().cloned())
        .collect();
    Ok(Scan {
        searched,
        files: library.into_iter().chain(elsewhere).collect(),
    })
}

/// A configuration file, by the end of its name: `*.vz.yml` or `*.vz.yaml`;
/// and the plain `vz.yml`.
pub fn is_config_file(file_name: &str) -> bool {
    file_name == SHORT_CONFIG_FILE
        || CONFIG_FILE_SUFFIXES
            .iter()
            .any(|suffix| file_name.ends_with(suffix))
}

/// The folder's configuration files, by filename.
fn read_folder(dir: &Path, scope: Scope, files: &impl Files) -> anyhow::Result<Vec<Scanned>> {
    let mut names = files.list(dir)?;
    names.sort();
    names
        .into_iter()
        .filter(|name| is_config_file(name))
        .map(|name| read(source(dir.join(name), scope), files))
        .collect()
}

fn source(path: PathBuf, scope: Scope) -> Source {
    Source { path, scope }
}

fn read(source: Source, files: &impl Files) -> anyhow::Result<Scanned> {
    let text = files
        .read(&source.path)
        .with_context(|| format!("reading {}", source.path.display()))?;
    Ok(Scanned { source, text })
}

/// The folders the library folder's files list under `scan:`, in order,
/// each once: relative to the file's folder, `~/…`, or absolute.
fn listed_folders(library: &[Scanned], folders: &Folders) -> anyhow::Result<Vec<PathBuf>> {
    let mut listed: Vec<PathBuf> = Vec::new();
    for file in library {
        for written in scan_key(&file.text) {
            let dir =
                paths::host_path(&written, file.source.dir(), folders.home, folders.repo_root)
                    .with_context(|| format!("in {}", file.source.path.display()))?;
            if dir != folders.library && !listed.contains(&dir) {
                listed.push(dir);
            }
        }
    }
    Ok(listed)
}

/// Refuses `scan:` in any file outside the library folder.
fn refuse_scan(files: &[Scanned], library: &Path) -> anyhow::Result<()> {
    match files.iter().find(|file| !scan_key(&file.text).is_empty()) {
        Some(file) => bail!(
            "in {}: `scan` belongs in a file of {}",
            file.source.path.display(),
            library.display()
        ),
        None => Ok(()),
    }
}

/// The `scan:` of every document of a file, read leniently: a file that does
/// not parse lists nothing here, and the parse step reports it.
fn scan_key(text: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct ScanKey {
        #[serde(default)]
        scan: Vec<String>,
    }
    serde_saphyr::from_multiple::<ScanKey>(text)
        .map(|documents| documents.into_iter().flat_map(|key| key.scan).collect())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;
    use crate::config::testing::{FakeFiles, HOME, LIBRARY, REPO, message};

    /// A scan of `files` from Sally's `~/repos/app`, with these `-f` files.
    fn scan(files: &[(&str, &str)], extra_files: &[&str]) -> anyhow::Result<Scan> {
        let extra_files: Vec<PathBuf> = extra_files.iter().map(PathBuf::from).collect();
        let folders = Folders {
            library: Path::new(LIBRARY),
            repo_root: Path::new(REPO),
            extra_files: &extra_files,
            home: Path::new(HOME),
        };
        collect(&folders, &FakeFiles::with(files))
    }

    fn paths(scan: &Scan) -> Vec<&Path> {
        scan.files
            .iter()
            .map(|file| file.source.path.as_path())
            .collect()
    }

    const APP_FILE: &str = "/home/sally/repos/app/app.vz.yml";

    /// A file in every place a scan looks: the library, a folder it lists,
    /// the repository root, and an `-f` file.
    fn every_place() -> [(&'static str, &'static str); 4] {
        [
            (APP_FILE, "image: debian\n"),
            ("/home/sally/team/x.vz.yml", "name: x\n"),
            (
                "/home/sally/.config/viz-shell/default.vz.yml",
                "scan: [~/team]\n",
            ),
            ("/srv/ci/ci.yml", "name: ci\n"),
        ]
    }

    #[test]
    fn collect__every_place__files_from_library_its_folders_repository_then_f_files() {
        let files = every_place();

        let scan = scan(&files, &["/srv/ci/ci.yml"]).unwrap();

        let expected = [
            Path::new("/home/sally/.config/viz-shell/default.vz.yml"),
            Path::new("/home/sally/team/x.vz.yml"),
            Path::new(APP_FILE),
            Path::new("/srv/ci/ci.yml"),
        ];
        assert_eq!(paths(&scan), expected);
    }

    #[test]
    fn collect__every_place__searched_library_its_folders_repository_then_f_files() {
        let files = every_place();

        let scan = scan(&files, &["/srv/ci/ci.yml"]).unwrap();

        let expected = [
            PathBuf::from(LIBRARY),
            PathBuf::from("/home/sally/team"),
            PathBuf::from(REPO),
            PathBuf::from("/srv/ci/ci.yml"),
        ];
        assert_eq!(scan.searched, expected);
    }

    #[test]
    fn is_config_file__the_two_suffixes_or_plain_vz_yml__accepted() {
        let accepted = [
            "app.vz.yml",
            "app.vz.yaml",
            "default.vz.yml",
            ".sally.vz.yml",
            "vz.yml",
        ];
        for file_name in accepted {
            let result = is_config_file(file_name);

            assert!(result, "{file_name}");
        }
    }

    #[test]
    fn is_config_file__any_other_name__rejected() {
        let rejected = [
            "vz.yaml",
            "viz-shell.yml",
            "viz-shell.global.yml",
            "global.yml",
            "app.yml",
            "app.vz.yml.bak",
            "Dockerfile",
        ];
        for file_name in rejected {
            let result = is_config_file(file_name);

            assert!(!result, "{file_name}");
        }
    }

    #[test]
    fn collect__a_folder__its_configuration_files_by_filename() {
        let files = [
            ("/home/sally/repos/app/viz-shell.yml", "{}\n"),
            ("/home/sally/repos/app/b.vz.yml", "{}\n"),
            ("/home/sally/repos/app/README.md", "{}\n"),
            ("/home/sally/repos/app/a.vz.yaml", "{}\n"),
            ("/home/sally/repos/app/Dockerfile", "{}\n"),
        ];

        let scan = scan(&files, &[]).unwrap();

        let expected = [
            Path::new("/home/sally/repos/app/a.vz.yaml"),
            Path::new("/home/sally/repos/app/b.vz.yml"),
        ];
        assert_eq!(paths(&scan), expected);
    }

    #[test]
    fn collect__f_file_of_any_name__a_repository_file() {
        let files = [("/srv/ci/other.yml", "{}\n")];

        let scan = scan(&files, &["/srv/ci/other.yml"]).unwrap();

        let found: Vec<(&Path, Scope)> = scan
            .files
            .iter()
            .map(|file| (file.source.path.as_path(), file.source.scope))
            .collect();
        assert_eq!(found, [(Path::new("/srv/ci/other.yml"), Scope::Repository)]);
    }

    #[test]
    fn collect__f_file_in_the_repository_root__read_once() {
        let files = [(APP_FILE, "image: debian\n")];

        let scan = scan(&files, &[APP_FILE]).unwrap();

        assert_eq!(paths(&scan), [Path::new(APP_FILE)]);
    }

    #[test]
    fn collect__scan_folders__in_listed_order_each_once_from_any_document() {
        let files = [
            (
                "/home/sally/.config/viz-shell/default.vz.yml",
                "scan: [~/b, /srv/a]\n---\nname: x\nscan: [c]\n",
            ),
            (
                "/home/sally/.config/viz-shell/trusted.vz.yml",
                "scan: [~/b]\n",
            ),
        ];

        let scan = scan(&files, &[]).unwrap();

        let expected = [
            PathBuf::from(LIBRARY),
            PathBuf::from("/home/sally/b"),
            PathBuf::from("/srv/a"),
            PathBuf::from("/home/sally/.config/viz-shell/c"),
            PathBuf::from(REPO),
        ];
        assert_eq!(scan.searched, expected);
    }

    #[test]
    fn collect__scan_outside_the_library_folder__refused_naming_the_file() {
        let library = (
            "/home/sally/.config/viz-shell/default.vz.yml",
            "scan: [~/team]\n",
        );
        let clean = (APP_FILE, "image: debian\n");
        let cases = [
            (
                "a repository file",
                vec![library, (APP_FILE, "scan: [~/x]\n")],
                vec![],
                "in /home/sally/repos/app/app.vz.yml: \
                 `scan` belongs in a file of /home/sally/.config/viz-shell",
            ),
            (
                "a file of a folder the library lists",
                vec![library, ("/home/sally/team/x.vz.yml", "scan: [~/x]\n")],
                vec![],
                "in /home/sally/team/x.vz.yml: \
                 `scan` belongs in a file of /home/sally/.config/viz-shell",
            ),
            (
                "a repository file after a clean one",
                vec![
                    library,
                    clean,
                    ("/home/sally/repos/app/z.vz.yml", "scan: [~/x]\n"),
                ],
                vec![],
                "in /home/sally/repos/app/z.vz.yml: \
                 `scan` belongs in a file of /home/sally/.config/viz-shell",
            ),
            (
                "an -f file after a clean repository file",
                vec![library, clean, ("/srv/ci/ci.yml", "scan: [~/x]\n")],
                vec!["/srv/ci/ci.yml"],
                "in /srv/ci/ci.yml: `scan` belongs in a file of /home/sally/.config/viz-shell",
            ),
        ];
        for (case, files, extra_files, expected) in cases {
            let message = message(scan(&files, &extra_files));

            assert!(message.contains(expected), "{case}: {message}");
        }
    }

    #[test]
    fn collect__missing_f_file__refused_naming_it() {
        let message = message(scan(&[], &["/srv/ci/ci.yml"]));

        assert!(message.contains("reading /srv/ci/ci.yml"), "{message}");
    }
}
