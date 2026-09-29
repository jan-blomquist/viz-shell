//! State: paths inside the container whose contents outlive it. Each is kept
//! under the state folder at its own container path, so `~/.local/share/opencode` lives at
//! `.vz_state/home/sally/.local/share/opencode`, and bind-mounted back.

use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};

use crate::config::{StateEntry, StateKind, expand_path};

#[derive(Debug, Clone, PartialEq)]
pub struct StateMount {
    /// On the host, under the cache folder.
    pub source: PathBuf,
    /// Inside the container.
    pub target: PathBuf,
    pub kind: StateKind,
    pub init: Option<String>,
}

/// Where each entry is mounted and stored. Refuses a path that holds the
/// repository or lies inside it: mounting either would make the engine
/// create root-owned folders on the host.
/// Each entry is kept under `state_dir` at its container path.
pub fn plan(
    entries: &[StateEntry],
    home: &Path,
    state_dir: &Path,
    repo_root: &Path,
) -> anyhow::Result<Vec<StateMount>> {
    entries
        .iter()
        .map(|entry| {
            let target = expand_path(&entry.path, home);
            ensure!(
                !repo_root.starts_with(&target) && !target.starts_with(repo_root),
                "state path `{}` overlaps the repository at {}",
                entry.path,
                repo_root.display()
            );
            Ok(StateMount {
                source: state_dir.join(target.strip_prefix("/").unwrap_or(&target)),
                target,
                kind: entry.kind,
                init: entry.init.clone(),
            })
        })
        .collect()
}

/// Creates each missing source as the host user: a folder, or a file with
/// its `init` content. An existing source is kept as it is.
pub fn create_sources(mounts: &[StateMount]) -> anyhow::Result<()> {
    for mount in mounts {
        let source = &mount.source;
        match mount.kind {
            StateKind::Dir => std::fs::create_dir_all(source),
            StateKind::File if source.exists() => Ok(()),
            StateKind::File => source
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(source, mount.init.as_deref().unwrap_or_default())),
        }
        .with_context(|| format!("creating {}", source.display()))?;
        let is_dir = source.is_dir();
        ensure!(
            is_dir == (mount.kind == StateKind::Dir),
            "{} is a {}, but vz.yml declares a {:?}",
            source.display(),
            if is_dir { "folder" } else { "file" },
            mount.kind
        );
    }
    Ok(())
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    const HOME: &str = "/home/sally";
    const REPO: &str = "/home/sally/repos/app";
    const STATE_DIR: &str = "/home/sally/repos/app/.vz_state";

    fn entry(path: &str, kind: StateKind, init: Option<&str>) -> StateEntry {
        StateEntry {
            path: path.to_owned(),
            kind,
            init: init.map(str::to_owned),
        }
    }

    #[test]
    fn plan__home_and_absolute_paths__mirrored_under_the_cache() {
        let entries = [
            entry("~/.local/share/opencode", StateKind::Dir, None),
            entry(
                "~/.config/opencode/opencode.json",
                StateKind::File,
                Some("{}"),
            ),
            entry("/var/cache/apt", StateKind::Dir, None),
        ];

        let mounts = plan(
            &entries,
            Path::new(HOME),
            Path::new(STATE_DIR),
            Path::new(REPO),
        )
        .unwrap();

        let mount = |source: &str, target: &str, kind, init: Option<&str>| StateMount {
            source: PathBuf::from(source),
            target: PathBuf::from(target),
            kind,
            init: init.map(str::to_owned),
        };
        let expected = vec![
            mount(
                "/home/sally/repos/app/.vz_state/home/sally/.local/share/opencode",
                "/home/sally/.local/share/opencode",
                StateKind::Dir,
                None,
            ),
            mount(
                "/home/sally/repos/app/.vz_state/home/sally/.config/opencode/opencode.json",
                "/home/sally/.config/opencode/opencode.json",
                StateKind::File,
                Some("{}"),
            ),
            mount(
                "/home/sally/repos/app/.vz_state/var/cache/apt",
                "/var/cache/apt",
                StateKind::Dir,
                None,
            ),
        ];
        assert_eq!(mounts, expected);
    }

    #[test]
    fn plan__path_overlapping_the_repository__is_refused() {
        let paths = ["~/repos", "~/repos/app", "~/repos/app/target"];
        for path in paths {
            let entries = [entry(path, StateKind::Dir, None)];

            let result = plan(
                &entries,
                Path::new(HOME),
                Path::new(STATE_DIR),
                Path::new(REPO),
            );

            assert!(result.is_err(), "path: {path}");
        }
    }

    fn file_mount(source: PathBuf, init: Option<&str>) -> StateMount {
        StateMount {
            source,
            target: PathBuf::from("/unused"),
            kind: StateKind::File,
            init: init.map(str::to_owned),
        }
    }

    #[test]
    fn create_sources__missing_file__created_with_init_and_parents() {
        let cache = tempfile::tempdir().unwrap();
        let source = cache
            .path()
            .join("home/sally/.config/opencode/opencode.json");

        create_sources(&[file_mount(source.clone(), Some("{}"))]).unwrap();

        assert_eq!(std::fs::read_to_string(&source).unwrap(), "{}");
    }

    #[test]
    fn create_sources__existing_file__kept_as_it_is() {
        let cache = tempfile::tempdir().unwrap();
        let source = cache.path().join("opencode.json");
        std::fs::write(&source, r#"{"kept":true}"#).unwrap();

        create_sources(&[file_mount(source.clone(), Some("{}"))]).unwrap();

        assert_eq!(
            std::fs::read_to_string(&source).unwrap(),
            r#"{"kept":true}"#
        );
    }

    #[test]
    fn create_sources__missing_folder__created() {
        let cache = tempfile::tempdir().unwrap();
        let source = cache.path().join("home/sally/.local/share/fish");
        let mount = StateMount {
            source: source.clone(),
            target: PathBuf::from("/unused"),
            kind: StateKind::Dir,
            init: None,
        };

        create_sources(&[mount]).unwrap();

        assert!(source.is_dir());
    }

    #[test]
    fn create_sources__folder_declared_as_file__is_refused() {
        let cache = tempfile::tempdir().unwrap();

        let result = create_sources(&[file_mount(cache.path().to_owned(), None)]);

        assert!(result.is_err());
    }

    #[test]
    fn plan__sibling_of_the_repository__is_accepted() {
        let entries = [entry("~/repos/other", StateKind::Dir, None)];

        let result = plan(
            &entries,
            Path::new(HOME),
            Path::new(STATE_DIR),
            Path::new(REPO),
        );

        assert!(result.is_ok(), "{result:?}");
    }
}
