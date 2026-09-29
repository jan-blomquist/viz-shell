//! Mounts: host paths shown at the same path inside the container, read-only
//! unless declared `rw`. Unlike state, they reuse the host's own files.

use std::path::{Path, PathBuf};

use anyhow::ensure;

use crate::config::{MountEntry, MountMode, expand_path};

#[derive(Debug, Clone, PartialEq)]
pub struct HostMount {
    /// The same on the host and inside.
    pub path: PathBuf,
    pub read_only: bool,
}

/// Refuses a mount that holds a state path or lies inside one: the engine
/// would create the inner mount point as root, in the host folder or in the
/// cache.
pub fn plan(
    entries: &[MountEntry],
    home: &Path,
    state_targets: &[PathBuf],
) -> anyhow::Result<Vec<HostMount>> {
    entries
        .iter()
        .map(|entry| {
            let path = expand_path(&entry.path, home);
            let overlapping = state_targets
                .iter()
                .find(|state| state.starts_with(&path) || path.starts_with(state));
            ensure!(
                overlapping.is_none(),
                "mount `{}` overlaps state path {}",
                entry.path,
                overlapping
                    .map(|state| state.display().to_string())
                    .unwrap_or_default()
            );
            Ok(HostMount {
                path,
                read_only: entry.mode == MountMode::Ro,
            })
        })
        .collect()
}

/// A mount shows an existing host path; vz never creates one.
pub fn check_sources_exist(mounts: &[HostMount]) -> anyhow::Result<()> {
    for mount in mounts {
        ensure!(
            mount.path.exists(),
            "mount {} does not exist on the host",
            mount.path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    const HOME: &str = "/home/sally";

    fn entry(path: &str, mode: MountMode) -> MountEntry {
        MountEntry {
            path: path.to_owned(),
            mode,
        }
    }

    #[test]
    fn plan__entries__expanded_with_read_only_flag() {
        let entries = [
            entry("~/repos", MountMode::Ro),
            entry("/opt/tools", MountMode::Rw),
        ];

        let mounts = plan(&entries, Path::new(HOME), &[]).unwrap();

        let expected = vec![
            HostMount {
                path: PathBuf::from("/home/sally/repos"),
                read_only: true,
            },
            HostMount {
                path: PathBuf::from("/opt/tools"),
                read_only: false,
            },
        ];
        assert_eq!(mounts, expected);
    }

    #[test]
    fn plan__overlapping_a_state_path__is_refused() {
        let state = [PathBuf::from("/home/sally/.config/gh")];
        let paths = ["~/.config", "~/.config/gh", "~/.config/gh/hosts"];
        for path in paths {
            let entries = [entry(path, MountMode::Ro)];

            let result = plan(&entries, Path::new(HOME), &state);

            assert!(result.is_err(), "path: {path}");
        }
    }

    #[test]
    fn plan__beside_a_state_path__is_accepted() {
        let state = [PathBuf::from("/home/sally/.config/gh")];
        let entries = [entry("~/.config/git", MountMode::Ro)];

        let result = plan(&entries, Path::new(HOME), &state);

        assert!(result.is_ok(), "{result:?}");
    }
}
