//! The project vz runs for: a git work tree, else a folder holding a
//! configuration, else, where the configuration allows it, the folder vz
//! runs in.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail, ensure};
use tracing::debug;

use crate::config::{Files, RealFiles, is_config_file};
use crate::constants::LIBRARY_BASE;

/// The root of the project, and what marks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    /// The top of the git work tree around the current directory.
    Git(PathBuf),
    /// The nearest folder, the current one or above, holding a configuration file.
    Configured(PathBuf),
    /// Neither: the current directory.
    Unmarked(PathBuf),
}

impl Root {
    pub fn path(&self) -> &Path {
        match self {
            Root::Git(path) | Root::Configured(path) | Root::Unmarked(path) => path,
        }
    }

    pub fn is_project(&self) -> bool {
        !matches!(self, Root::Unmarked(_))
    }

    /// Refuses `/`, which cannot be mounted over the container's root, and,
    /// with `require_project`, a folder outside any project.
    pub fn check(&self, require_project: bool) -> anyhow::Result<()> {
        ensure!(
            self.path() != Path::new("/"),
            "the project root is /, which cannot be mounted over the container's root; \
             run vz in a folder below it"
        );
        if require_project && !self.is_project() {
            bail!(
                "not in a project: no .git or vz.yml here or above\n  \
                 git init, or: echo 'extends: {LIBRARY_BASE}' > vz.yml; \
                 or require_project: false in a library configuration"
            );
        }
        Ok(())
    }
}

/// The project around the current directory: git's work tree, else the
/// nearest folder holding a configuration file, else the current directory.
pub fn root() -> anyhow::Result<Root> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let ceilings = ceilings(std::env::var_os("GIT_CEILING_DIRECTORIES").as_deref());
    let has_config = |dir: &Path| {
        RealFiles
            .list(dir)
            .is_ok_and(|names| names.iter().any(|name| is_config_file(name)))
    };
    Ok(find_root(&cwd, git_toplevel(), &ceilings, &has_config))
}

/// The project root of `cwd`: `git_top` when git found a work tree; else
/// the nearest folder, `cwd` or above, `has_config`, the walk stopping
/// below each of `ceilings` as git's does; else `cwd`.
pub fn find_root(
    cwd: &Path,
    git_top: Option<PathBuf>,
    ceilings: &[PathBuf],
    has_config: &dyn Fn(&Path) -> bool,
) -> Root {
    if let Some(top) = git_top {
        return Root::Git(top);
    }
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        if has_config(current) {
            return Root::Configured(current.to_owned());
        }
        dir = current
            .parent()
            .filter(|parent| !ceilings.iter().any(|ceiling| ceiling == parent));
    }
    Root::Unmarked(cwd.to_owned())
}

/// `GIT_CEILING_DIRECTORIES`: absolute folders, `:`-separated, the walk
/// never goes up into.
fn ceilings(value: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    value
        .map(|value| {
            std::env::split_paths(value)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .unwrap_or_default()
}

/// The top of the git work tree around the current directory; none outside
/// one, or without git.
fn git_toplevel() -> Option<PathBuf> {
    let output = match Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            debug!("no git: {error}");
            return None;
        }
    };
    let root = String::from_utf8(output.stdout).ok()?;
    output
        .status
        .success()
        .then(|| PathBuf::from(root.trim_end()))
}

/// Whether git tracks the file, in the work tree around it. False when it
/// is in none, or git fails.
pub fn is_tracked(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The branch checked out at `root`; `None` on a detached HEAD, or if git fails.
pub fn branch(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["symbolic-ref", "--short", "-q", "HEAD"])
        .output()
        .ok()?;
    let branch = String::from_utf8(output.stdout).ok()?;
    Some(branch.trim_end().to_owned())
        .filter(|branch| output.status.success() && !branch.is_empty())
}

pub fn dir_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    fn root_with(files: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for file in files {
            std::fs::write(root.path().join(file), "").unwrap();
        }
        root
    }

    /// Runs git in `dir`; a failure is a broken arrange, not a result.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A git repository holding `tracked.yml`, added, and `untracked.yml`.
    fn repository() -> tempfile::TempDir {
        let repo = root_with(&["tracked.yml", "untracked.yml"]);
        git(repo.path(), &["init", "-q"]);
        git(repo.path(), &["add", "tracked.yml"]);
        repo
    }

    #[test]
    fn is_tracked__a_file_git_tracks__true() {
        let repo = repository();

        let tracked = is_tracked(&repo.path().join("tracked.yml"));

        assert!(tracked);
    }

    #[test]
    fn is_tracked__untracked_missing_or_outside_a_repository__false() {
        let repo = repository();
        let outside = root_with(&["loose.yml"]);
        let cases = [
            ("untracked", repo.path().join("untracked.yml")),
            ("missing", repo.path().join("missing.yml")),
            ("outside a repository", outside.path().join("loose.yml")),
        ];
        for (case, path) in cases {
            let tracked = is_tracked(&path);

            assert!(!tracked, "{case}");
        }
    }

    /// A walk from `cwd` over folders holding a configuration file, below
    /// these ceilings, without git.
    fn walk(cwd: &str, configured: &[&str], ceilings: &[&str]) -> Root {
        let has_config = |dir: &Path| configured.iter().any(|folder| Path::new(folder) == dir);
        let ceilings: Vec<PathBuf> = ceilings.iter().map(PathBuf::from).collect();
        find_root(Path::new(cwd), None, &ceilings, &has_config)
    }

    #[test]
    fn find_root__a_git_work_tree__its_top_even_with_a_configuration_below() {
        let has_config = |_: &Path| true;

        let root = find_root(
            Path::new("/home/sally/repos/app/sub"),
            Some(PathBuf::from("/home/sally/repos/app")),
            &[],
            &has_config,
        );

        assert_eq!(root, Root::Git(PathBuf::from("/home/sally/repos/app")));
    }

    #[test]
    fn find_root__no_git__the_nearest_folder_holding_a_configuration_else_unmarked() {
        let cwd = "/home/sally/repos/app/sub";
        let cases = [
            (
                "here",
                vec![cwd, "/home/sally/repos/app"],
                vec![],
                Root::Configured(PathBuf::from(cwd)),
            ),
            (
                "above",
                vec!["/home/sally/repos/app"],
                vec![],
                Root::Configured(PathBuf::from("/home/sally/repos/app")),
            ),
            (
                "at the top",
                vec!["/"],
                vec![],
                Root::Configured(PathBuf::from("/")),
            ),
            ("none", vec![], vec![], Root::Unmarked(PathBuf::from(cwd))),
            (
                "only at a ceiling",
                vec!["/home/sally/repos"],
                vec!["/home/sally/repos"],
                Root::Unmarked(PathBuf::from(cwd)),
            ),
            (
                "below a ceiling",
                vec!["/home/sally/repos/app"],
                vec!["/home/sally/repos"],
                Root::Configured(PathBuf::from("/home/sally/repos/app")),
            ),
        ];
        for (case, configured, ceilings, expected) in cases {
            let root = walk(cwd, &configured, &ceilings);

            assert_eq!(root, expected, "{case}");
        }
    }

    #[test]
    fn ceilings__git_ceiling_directories__the_absolute_ones() {
        let value = std::ffi::OsString::from("/home/sally:relative::/srv");

        let ceilings = ceilings(Some(&value));

        assert_eq!(
            ceilings,
            [PathBuf::from("/home/sally"), PathBuf::from("/srv")]
        );
    }

    #[test]
    fn check__a_project_or_unmarked_allowed__accepted() {
        let cases = [
            (
                "git",
                Root::Git(PathBuf::from("/home/sally/repos/app")),
                true,
            ),
            (
                "configured",
                Root::Configured(PathBuf::from("/home/sally/repos/app")),
                true,
            ),
            (
                "unmarked, allowed",
                Root::Unmarked(PathBuf::from("/home/sally/Downloads")),
                false,
            ),
        ];
        for (case, root, require_project) in cases {
            let checked = root.check(require_project);

            assert!(checked.is_ok(), "{case}: {checked:?}");
        }
    }

    #[test]
    fn check__unmarked_with_require_project__refused_saying_what_to_do() {
        let root = Root::Unmarked(PathBuf::from("/home/sally/Downloads"));

        let message = format!("{:#}", root.check(true).unwrap_err());

        assert_eq!(
            message,
            "not in a project: no .git or vz.yml here or above\n  \
             git init, or: echo 'extends: vz-debian-trixie' > vz.yml; \
             or require_project: false in a library configuration"
        );
    }

    #[test]
    fn check__root_of_slash__refused_whatever_require_project() {
        let cases = [
            ("git", Root::Git(PathBuf::from("/")), true),
            ("configured", Root::Configured(PathBuf::from("/")), true),
            (
                "unmarked, allowed",
                Root::Unmarked(PathBuf::from("/")),
                false,
            ),
        ];
        for (case, root, require_project) in cases {
            let message = format!("{:#}", root.check(require_project).unwrap_err());

            assert!(
                message.contains("the project root is /, which cannot be mounted"),
                "{case}: {message}"
            );
        }
    }

    #[test]
    fn branch__outside_a_repository__none() {
        let outside = root_with(&[]);

        let branch = branch(outside.path());

        assert_eq!(branch, None);
    }
}
