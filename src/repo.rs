//! The git repository vz runs for.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, ensure};

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
}
