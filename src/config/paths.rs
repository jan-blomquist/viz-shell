//! Step 3, paths: every host path of a parsed file made absolute, right
//! after parsing: relative to the file's own folder, `~/…` under the home,
//! `${repo}` and `${home}` substituted. Invariant: once resolved, entries of
//! different files match by path, however each file wrote it.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use super::merge::Keyed;
use super::parse::{ImageSource, Layer, MountItem};
use crate::constants::HOME_PREFIX;

/// Resolves the layer's host paths against `dir`, the file's folder.
pub fn resolve(layer: &mut Layer, dir: &Path, home: &Path, repo_root: &Path) -> anyhow::Result<()> {
    let host = |path: &str| -> anyhow::Result<String> {
        Ok(host_path(path, dir, home, repo_root)?
            .to_string_lossy()
            .into_owned())
    };
    let expand = |path: &str| expand_path(path, home).to_string_lossy().into_owned();
    if let Some(ImageSource::Build(spec)) = &mut layer.image {
        spec.dockerfile = host_path(&spec.dockerfile.to_string_lossy(), dir, home, repo_root)?;
        spec.context = host_path(&spec.context.to_string_lossy(), dir, home, repo_root)?;
    }
    if let Some(state_dir) = &layer.state_dir {
        layer.state_dir = Some(host(state_dir)?);
    }
    for entry in &mut layer.env.files {
        let path = host(entry.key())?;
        *entry.key_mut() = path;
    }
    for entry in &mut layer.state {
        let path = expand(entry.key());
        *entry.key_mut() = path;
    }
    for entry in &mut layer.mounts {
        match entry {
            MountItem::Path(path) => *path = expand(path),
            MountItem::Full(spec) => {
                spec.path = expand(&spec.path);
                spec.target = spec.target.as_deref().map(expand);
            }
        }
    }
    Ok(())
}

/// A host path as a file writes it, absolute: `${repo}` and `${home}`
/// substituted, then [`resolve_host_path`].
pub fn host_path(path: &str, dir: &Path, home: &Path, repo_root: &Path) -> anyhow::Result<PathBuf> {
    let path = substitute(path, repo_root, home)?;
    Ok(resolve_host_path(&path, dir, home))
}

/// `${repo}` and `${home}`; any other `${…}` is an error. A `$` not followed
/// by `{` stays as it is.
pub fn substitute(text: &str, repo_root: &Path, home: &Path) -> anyhow::Result<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .with_context(|| format!("`{text}` has a `${{` without `}}`"))?;
        match &after[..end] {
            "repo" => out.push_str(&repo_root.to_string_lossy()),
            "home" => out.push_str(&home.to_string_lossy()),
            other => bail!("`${{{other}}}` in `{text}`: vz substitutes ${{repo}} and ${{home}}"),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A path written for the host: `~/…` under the home, absolute as it is,
/// anything else relative to `base`, the file's folder.
pub fn resolve_host_path(path: &str, base: &Path, home: &Path) -> PathBuf {
    if path.starts_with(HOME_PREFIX) || path.starts_with('/') {
        expand_path(path, home)
    } else {
        base.join(path)
    }
}

/// `~/x` under the home; an absolute path as it is. The home has the same
/// path inside the container as on the host.
pub fn expand_path(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix(HOME_PREFIX) {
        Some(below_home) => home.join(below_home),
        None => PathBuf::from(path),
    }
}

/// A path under the home as `~/…`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(below) => format!("~/{}", below.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A path as vz prints it: relative to the repository root when inside it,
/// else [`tilde`].
pub fn shown(path: &Path, repo_root: &Path, home: &Path) -> String {
    match path.strip_prefix(repo_root) {
        Ok(inside) => inside.display().to_string(),
        Err(_) => tilde(path, home),
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;
    use crate::config::parse::{FileItem, Layer, parse};
    use crate::config::testing::{HOME, REPO, message};

    /// The one document of `text`.
    fn one(text: &str) -> Layer {
        parse(text).unwrap().remove(0)
    }

    #[test]
    fn resolve__a_file_s_paths__absolute_against_its_own_folder() {
        let text = "\
image: { dockerfile: base.Dockerfile }
state_dir: ${repo}/.state
env:
  files: [environment, ~/.env, /etc/env]
mounts: [~/repos, { path: ~/skills, target: ~/.agents/skills }]
state: [~/.cache]
";
        let mut layer = one(text);
        let library = Path::new("/home/sally/.config/viz-shell");

        resolve(&mut layer, library, Path::new(HOME), Path::new(REPO)).unwrap();

        let resolved = one("\
image:
  dockerfile: /home/sally/.config/viz-shell/base.Dockerfile
  context: /home/sally/.config/viz-shell/.
state_dir: /home/sally/repos/app/.state
env:
  files:
    - /home/sally/.config/viz-shell/environment
    - /home/sally/.env
    - /etc/env
mounts: [/home/sally/repos, { path: /home/sally/skills, target: /home/sally/.agents/skills }]
state: [/home/sally/.cache]
");
        assert_eq!(layer, resolved);
    }

    #[test]
    fn resolve__image_paths__relative_home_absolute_and_substituted() {
        let cases = [
            (
                "relative",
                "image: { dockerfile: Dockerfile, context: . }\n",
                "image: { dockerfile: /home/sally/repos/app/Dockerfile, context: /home/sally/repos/app/. }\n",
            ),
            (
                "relative, deeper",
                "image: { dockerfile: images/tools/Dockerfile, context: images/tools }\n",
                "image: { dockerfile: /home/sally/repos/app/images/tools/Dockerfile, \
                 context: /home/sally/repos/app/images/tools }\n",
            ),
            (
                "under the home",
                "image: { dockerfile: ~/repos/tools/Dockerfile, context: ~/repos/tools }\n",
                "image: { dockerfile: /home/sally/repos/tools/Dockerfile, \
                 context: /home/sally/repos/tools }\n",
            ),
            (
                "absolute",
                "image: { dockerfile: /srv/images/Dockerfile, context: /srv/images }\n",
                "image: { dockerfile: /srv/images/Dockerfile, context: /srv/images }\n",
            ),
            (
                "${home}",
                "image: { dockerfile: \"${home}/repos/tools/Dockerfile\", context: \"${home}/repos/tools\" }\n",
                "image: { dockerfile: /home/sally/repos/tools/Dockerfile, \
                 context: /home/sally/repos/tools }\n",
            ),
            (
                "${repo}",
                "image: { dockerfile: \"${repo}/Dockerfile\", context: \"${repo}\" }\n",
                "image: { dockerfile: /home/sally/repos/app/Dockerfile, context: /home/sally/repos/app }\n",
            ),
        ];
        for (case, written, expected) in cases {
            let mut layer = one(written);

            resolve(
                &mut layer,
                Path::new(REPO),
                Path::new(HOME),
                Path::new(REPO),
            )
            .unwrap();

            assert_eq!(layer, one(expected), "{case}");
        }
    }

    #[test]
    fn resolve__unknown_substitution__refused() {
        let mut layer = one("env:\n  files: [\"${config}/x\"]\n");

        let message = message(resolve(
            &mut layer,
            Path::new(REPO),
            Path::new(HOME),
            Path::new(REPO),
        ));

        assert!(message.contains("${config}"), "{message}");
    }

    #[test]
    fn resolve__env_file_written_two_ways__one_key() {
        let resolved = |text: &str, dir: &str| {
            let mut layer = one(text);
            resolve(&mut layer, Path::new(dir), Path::new(HOME), Path::new(REPO)).unwrap();
            layer.env.files
        };

        let relative = resolved("env:\n  files: [.env]\n", REPO);
        let from_home = resolved("env:\n  files: [~/repos/app/.env]\n", "/elsewhere");

        let expected = vec![FileItem::Path("/home/sally/repos/app/.env".to_owned())];
        assert_eq!((relative, from_home), (expected.clone(), expected));
    }

    #[test]
    fn substitute__repo_home_or_a_bare_dollar__substituted_or_kept() {
        let cases = [
            ("plain", "plain"),
            ("${repo}/data", "/home/sally/repos/app/data"),
            ("${home}/.cache", "/home/sally/.cache"),
            ("cost: $5 and $HOME", "cost: $5 and $HOME"),
        ];
        for (text, expected) in cases {
            let substituted = substitute(text, Path::new(REPO), Path::new(HOME)).unwrap();

            assert_eq!(substituted, expected, "text: {text}");
        }
    }

    #[test]
    fn substitute__unknown_or_unclosed__is_refused_naming_it() {
        for (text, expected) in [
            ("${config}", "`${config}` in `${config}`"),
            ("${env:X}", "`${env:X}` in `${env:X}`"),
            ("${repo", "`${repo` has a `${` without `}`"),
        ] {
            let message = message(substitute(text, Path::new(REPO), Path::new(HOME)));

            assert!(message.contains(expected), "{text}: {message}");
        }
    }

    #[test]
    fn resolve_host_path__relative_home_and_absolute() {
        let cases = [
            (
                ".vz_state",
                "/home/sally/repos/app/examples/state/.vz_state",
            ),
            ("~/.cache/vz", "/home/sally/.cache/vz"),
            ("/var/cache/vz", "/var/cache/vz"),
        ];
        for (path, expected) in cases {
            let resolved = resolve_host_path(
                path,
                Path::new("/home/sally/repos/app/examples/state"),
                Path::new(HOME),
            );

            assert_eq!(resolved, PathBuf::from(expected), "path: {path}");
        }
    }

    #[test]
    fn shown__paths__relative_in_the_repository_else_tilde_else_in_full() {
        let cases = [
            ("/home/sally/repos/app/app.vz.yml", "app.vz.yml"),
            ("/home/sally/repos/app/ci/ci.vz.yml", "ci/ci.vz.yml"),
            (
                "/home/sally/.config/viz-shell/default.vz.yml",
                "~/.config/viz-shell/default.vz.yml",
            ),
            ("/srv/ci/ci.vz.yml", "/srv/ci/ci.vz.yml"),
        ];
        for (path, expected) in cases {
            let shown = shown(Path::new(path), Path::new(REPO), Path::new(HOME));

            assert_eq!(shown, expected, "{path}");
        }
    }
}
