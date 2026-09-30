//! Mounts: host paths shown inside the container, at the same path unless a
//! `target` says otherwise, read-only unless declared `rw`. Unlike state,
//! they reuse the host's own files.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use tracing::debug;

use crate::config::{MountEntry, MountMode, StateKind, expand_path};
use crate::state::StateMount;

#[derive(Debug, Clone, PartialEq)]
pub struct HostMount {
    /// On the host.
    pub source: PathBuf,
    /// Inside the container.
    pub target: PathBuf,
    pub read_only: bool,
    /// When it lands inside a state folder: its mount point there, which vz
    /// creates as the user so the engine does not create it as root.
    pub point_in_state: Option<PathBuf>,
}

/// A mount may land inside a state folder, but not hold one or replace one:
/// the engine would then create the state's mount point as root, in the host
/// folder, or a state path would hide the mount.
///
/// A mount that lands on the repository vz runs for is skipped: the
/// repository is mounted there already, read-write. So a read-write
/// `~/repos/kb` inside a read-only `~/repos` serves every session, kb's own
/// included.
pub fn plan(
    entries: &[MountEntry],
    home: &Path,
    state: &[StateMount],
    repo_root: &Path,
) -> anyhow::Result<Vec<HostMount>> {
    entries
        .iter()
        .filter(|entry| {
            let target = expand_path(entry.target.as_deref().unwrap_or(&entry.path), home);
            let on_the_repository = target == repo_root;
            if on_the_repository {
                debug!(
                    "skipping mount {}: it lands on the repository, which is mounted read-write",
                    target.display()
                );
            }
            !on_the_repository
        })
        .map(|entry| {
            let source = expand_path(&entry.path, home);
            let target = entry
                .target
                .as_deref()
                .map_or_else(|| source.clone(), |target| expand_path(target, home));
            let mut point_in_state = None;
            for held in state {
                if held.target.starts_with(&target) {
                    bail!(
                        "mount at {} holds state path {}",
                        target.display(),
                        held.target.display()
                    );
                }
                if let Ok(below) = target.strip_prefix(&held.target) {
                    ensure!(
                        held.kind == StateKind::Dir,
                        "mount at {} lies inside state file {}",
                        target.display(),
                        held.target.display()
                    );
                    point_in_state = Some(held.source.join(below));
                }
            }
            Ok(HostMount {
                source,
                target,
                read_only: entry.mode == MountMode::Ro,
                point_in_state,
            })
        })
        .collect()
}

/// A mount shows an existing host path; vz never creates one.
pub fn check_sources_exist(mounts: &[HostMount]) -> anyhow::Result<()> {
    for mount in mounts {
        ensure!(
            mount.source.exists(),
            "mount {} does not exist on the host",
            mount.source.display()
        );
    }
    Ok(())
}

/// Creates each missing mount point inside a state folder, as the host user:
/// a folder for a folder, an empty file for a file. Run after the state
/// folders exist.
pub fn create_points_in_state(mounts: &[HostMount]) -> anyhow::Result<()> {
    for mount in mounts {
        let Some(point) = &mount.point_in_state else {
            continue;
        };
        if point.exists() {
            continue;
        }
        let created = match mount.source.is_dir() {
            true => std::fs::create_dir_all(point),
            false => point
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(point, "")),
        };
        created.with_context(|| format!("creating mount point {}", point.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    const HOME: &str = "/home/sally";
    const REPO: &str = "/home/sally/repos/app";

    fn entry(path: &str, target: Option<&str>, mode: MountMode) -> MountEntry {
        MountEntry {
            path: path.to_owned(),
            target: target.map(str::to_owned),
            mode,
        }
    }

    fn state_dir(target: &str) -> StateMount {
        StateMount {
            source: PathBuf::from("/home/sally/repos/app/.vz_state").join(&target[1..]),
            target: PathBuf::from(target),
            kind: StateKind::Dir,
            init: None,
        }
    }

    fn host(source: &str, target: &str, read_only: bool) -> HostMount {
        HostMount {
            source: PathBuf::from(source),
            target: PathBuf::from(target),
            read_only,
            point_in_state: None,
        }
    }

    #[test]
    fn plan__entries__expanded_same_path_unless_a_target() {
        let entries = [
            entry("~/repos", None, MountMode::Ro),
            entry("/opt/tools", None, MountMode::Rw),
            entry("~/skills", Some("~/.agents/skills"), MountMode::Ro),
        ];

        let mounts = plan(&entries, Path::new(HOME), &[], Path::new(REPO)).unwrap();

        let expected = vec![
            host("/home/sally/repos", "/home/sally/repos", true),
            host("/opt/tools", "/opt/tools", false),
            host("/home/sally/skills", "/home/sally/.agents/skills", true),
        ];
        assert_eq!(mounts, expected);
    }

    #[test]
    fn plan__landing_on_the_repository__skipped_others_kept() {
        let entries = [
            entry("~/repos", None, MountMode::Ro),
            entry("~/repos/app", None, MountMode::Rw),
            entry("~/elsewhere", Some("~/repos/app"), MountMode::Ro),
            entry("~/repos/app/sub", None, MountMode::Ro),
        ];

        let mounts = plan(&entries, Path::new(HOME), &[], Path::new(REPO)).unwrap();

        let targets: Vec<&Path> = mounts.iter().map(|mount| mount.target.as_path()).collect();
        assert_eq!(
            targets,
            [
                Path::new("/home/sally/repos"),
                Path::new("/home/sally/repos/app/sub")
            ]
        );
    }

    #[test]
    fn plan__inside_a_state_folder__its_mount_point_in_the_state_folder() {
        let state = [state_dir("/home/sally/.config/opencode")];
        let entries = [entry(
            "~/skills",
            Some("~/.config/opencode/skills"),
            MountMode::Ro,
        )];

        let mounts = plan(&entries, Path::new(HOME), &state, Path::new(REPO)).unwrap();

        assert_eq!(
            mounts[0].point_in_state,
            Some(PathBuf::from(
                "/home/sally/repos/app/.vz_state/home/sally/.config/opencode/skills"
            ))
        );
    }

    #[test]
    fn plan__holding_or_on_a_state_path_or_inside_a_state_file__is_refused() {
        let file = StateMount {
            kind: StateKind::File,
            ..state_dir("/home/sally/.gitconfig")
        };
        let state = [state_dir("/home/sally/.config/gh"), file];
        let targets = ["~/.config", "~/.config/gh", "~/.gitconfig/x"];
        for target in targets {
            let entries = [entry("~/x", Some(target), MountMode::Ro)];

            let result = plan(&entries, Path::new(HOME), &state, Path::new(REPO));

            assert!(result.is_err(), "target: {target}");
        }
    }

    #[test]
    fn plan__beside_a_state_path__is_accepted_without_a_point() {
        let state = [state_dir("/home/sally/.config/gh")];
        let entries = [entry("~/.config/git", None, MountMode::Ro)];

        let mounts = plan(&entries, Path::new(HOME), &state, Path::new(REPO)).unwrap();

        assert_eq!(mounts[0].point_in_state, None);
    }

    #[test]
    fn create_points_in_state__folder_or_file__created_once_as_the_user() {
        let root = tempfile::tempdir().unwrap();
        let dir_source = root.path().join("skills");
        let file_source = root.path().join("AGENTS.md");
        std::fs::create_dir(&dir_source).unwrap();
        std::fs::write(&file_source, "orientation").unwrap();
        let state = root.path().join("state/home/sally/.config/opencode");
        let mount = |source: &Path, name: &str| HostMount {
            source: source.to_owned(),
            target: PathBuf::from("/home/sally/.config/opencode").join(name),
            read_only: true,
            point_in_state: Some(state.join(name)),
        };
        let mounts = [
            mount(&dir_source, "skills"),
            mount(&file_source, "AGENTS.md"),
        ];

        create_points_in_state(&mounts).unwrap();
        std::fs::write(state.join("AGENTS.md"), "kept").unwrap();
        create_points_in_state(&mounts).unwrap();

        assert!(state.join("skills").is_dir());
        assert_eq!(
            std::fs::read_to_string(state.join("AGENTS.md")).unwrap(),
            "kept"
        );
    }
}
