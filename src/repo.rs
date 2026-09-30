//! The git repository vz runs for.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, ensure};
use tracing::warn;

use crate::constants::{LOCAL_CONFIG_FILES, LOCAL_SUFFIX, REPO_CONFIG_FILES};

/// The top of the git work tree around the current directory.
pub fn root() -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("running git, which vz needs to find the repository")?;
    ensure!(
        output.status.success(),
        "not inside a git work tree; run `git init`, or cd into a repository"
    );
    let root = String::from_utf8(output.stdout).context("git printed a non-UTF-8 path")?;
    Ok(PathBuf::from(root.trim_end()))
}

/// The repository's configuration file: the first of `REPO_CONFIG_FILES`
/// present at the root, if any. Others present are ignored, with a warning.
pub fn config_file(root: &Path) -> Option<PathBuf> {
    first_present(root, &REPO_CONFIG_FILES)
}

/// Your overlay of the repository's configuration: the first of
/// `LOCAL_CONFIG_FILES` present at the root, if any, by the same rule.
pub fn local_config_file(root: &Path) -> Option<PathBuf> {
    first_present(root, &LOCAL_CONFIG_FILES)
}

/// The overlay of a `-c` file: its name with `.local` before the extension,
/// next to it. `foo.yml` → `foo.local.yml`.
pub fn local_file_for(file: &Path) -> PathBuf {
    let stem = file.file_stem().unwrap_or_default().to_string_lossy();
    let name = match file.extension() {
        Some(extension) => format!("{stem}.{LOCAL_SUFFIX}.{}", extension.to_string_lossy()),
        None => format!("{stem}.{LOCAL_SUFFIX}"),
    };
    file.with_file_name(name)
}

/// The first of `names` present in `root`; others present are ignored,
/// with a warning.
pub fn first_present(root: &Path, names: &[&str]) -> Option<PathBuf> {
    let present: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| root.join(name).is_file())
        .collect();
    let (chosen, ignored) = present.split_first()?;
    if !ignored.is_empty() {
        warn!("using {chosen}; ignoring {}", ignored.join(", "));
    }
    Some(root.join(chosen))
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

    #[test]
    fn config_file__precedence() {
        let cases: [(&[&str], &str); 4] = [
            (&["vz.yml"], "vz.yml"),
            (&["vz.yaml", "vz.yml"], "vz.yml"),
            (&["vz.yml", "viz-shell.yaml"], "viz-shell.yaml"),
            (
                &["vz.yaml", "vz.yml", "viz-shell.yaml", "viz-shell.yml"],
                "viz-shell.yml",
            ),
        ];
        for (files, expected) in cases {
            let root = root_with(files);

            let chosen = config_file(root.path()).unwrap();

            assert_eq!(chosen, root.path().join(expected), "files: {files:?}");
        }
    }

    #[test]
    fn config_file__none__is_none() {
        let root = root_with(&[]);

        assert_eq!(config_file(root.path()), None);
    }

    #[test]
    fn local_config_file__each_repository_name__has_its_local_name() {
        for (repo, local) in REPO_CONFIG_FILES.into_iter().zip(LOCAL_CONFIG_FILES) {
            assert_eq!(local_file_for(Path::new(repo)), Path::new(local), "{repo}");
        }
    }

    #[test]
    fn local_file_for__cases() {
        let cases = [
            ("/r/foo.yml", "/r/foo.local.yml"),
            ("/r/foo.yaml", "/r/foo.local.yaml"),
            ("/r/configs/ci.vz.yml", "/r/configs/ci.vz.local.yml"),
            ("/r/foo", "/r/foo.local"),
        ];
        for (file, expected) in cases {
            assert_eq!(
                local_file_for(Path::new(file)),
                Path::new(expected),
                "{file}"
            );
        }
    }

    #[test]
    fn local_config_file__precedence() {
        let root = root_with(&["vz.local.yml", "viz-shell.local.yaml", "viz-shell.yml"]);

        let chosen = local_config_file(root.path()).unwrap();

        assert_eq!(chosen, root.path().join("viz-shell.local.yaml"));
    }

    #[test]
    fn is_tracked__cases() {
        let repo = root_with(&["tracked.yml", "untracked.yml"]);
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        git(&["add", "tracked.yml"]);
        let outside = root_with(&["loose.yml"]);
        let cases = [
            (repo.path().join("tracked.yml"), true),
            (repo.path().join("untracked.yml"), false),
            (repo.path().join("missing.yml"), false),
            (outside.path().join("loose.yml"), false),
        ];
        for (path, expected) in cases {
            assert_eq!(is_tracked(&path), expected, "{}", path.display());
        }
    }
}
