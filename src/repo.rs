//! The git repository vz runs for.

use std::path::{Path, PathBuf};
use std::process::Command;

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

pub fn dir_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
