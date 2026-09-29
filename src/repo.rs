//! The git repository vz runs for.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, ensure};
use tracing::warn;

use crate::constants::REPO_CONFIG_FILES;

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
/// present at the root. Others present are ignored, with a warning.
pub fn config_file(root: &Path) -> anyhow::Result<PathBuf> {
    let present: Vec<&str> = REPO_CONFIG_FILES
        .into_iter()
        .filter(|name| root.join(name).is_file())
        .collect();
    let (chosen, ignored) = present.split_first().with_context(|| {
        format!(
            "no configuration at {}: add one of {}",
            root.display(),
            REPO_CONFIG_FILES.join(", ")
        )
    })?;
    if !ignored.is_empty() {
        warn!("using {chosen}; ignoring {}", ignored.join(", "));
    }
    Ok(root.join(chosen))
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
    fn config_file__none__is_refused_naming_the_accepted_names() {
        let root = root_with(&[]);

        let error = config_file(root.path()).unwrap_err().to_string();

        assert!(
            error.contains("viz-shell.yml, viz-shell.yaml, vz.yml, vz.yaml"),
            "{error}"
        );
    }
}
