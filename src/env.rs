//! The environment inside the container: each variable's value, and where it
//! came from. Sources, later wins: `env.defaults`, `env.files` in order,
//! `env.passthrough`, then `vz --env`. Values are read on the host and reach
//! the container by name only: never on a command line, never in a log.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail, ensure};
use tracing::debug;

use crate::config::{EffectiveEnv, EnvFile, is_env_name, resolve_host_path};

/// Where a variable's value came from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Default,
    File(PathBuf),
    Host,
    Cli,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Default => write!(f, "env.defaults"),
            Source::File(path) => write!(f, "env.files {}", path.display()),
            Source::Host => write!(f, "env.passthrough"),
            Source::Cli => write!(f, "--env"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvVar {
    pub value: String,
    pub source: Source,
}

/// By name.
pub type Environment = BTreeMap<String, EnvVar>;

/// A `.env` file as read: its path on the host and its variables in order.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedFile {
    pub path: PathBuf,
    pub vars: Vec<(String, String)>,
}

/// `vz --env KEY=VALUE` sets a value; `vz --env KEY` copies the host's, as
/// docker's `-e` does.
#[derive(Debug, Clone, PartialEq)]
pub enum CliEnv {
    Value(String, String),
    FromHost(String),
}

impl CliEnv {
    pub fn parse(arg: &str) -> anyhow::Result<Self> {
        let (name, value) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (arg, None),
        };
        ensure!(
            is_env_name(name),
            "--env {arg}: `{name}` is not a variable name"
        );
        Ok(match value {
            Some(value) => CliEnv::Value(name.to_owned(), value.to_owned()),
            None => CliEnv::FromHost(name.to_owned()),
        })
    }
}

/// Where paths and `${…}` resolve.
pub struct Paths<'a> {
    pub config_dir: &'a Path,
    pub repo_root: &'a Path,
    pub home: &'a Path,
}

/// Reads what the environment needs from the host, then resolves it.
pub fn plan(env: &EffectiveEnv, cli: &[CliEnv], paths: &Paths) -> anyhow::Result<Environment> {
    let defaults = env
        .defaults
        .iter()
        .map(|(name, value)| {
            let value =
                substitute(value, paths).with_context(|| format!("in env default `{name}`"))?;
            Ok((name.clone(), value))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let files = load_files(&env.files, paths)?;
    let host: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    Ok(resolve(&defaults, &files, &env.passthrough, &host, cli))
}

/// The sources applied in order, each overriding what came before. Pure.
pub fn resolve(
    defaults: &[(String, String)],
    files: &[LoadedFile],
    passthrough: &[String],
    host: &[(String, String)],
    cli: &[CliEnv],
) -> Environment {
    let mut environment = Environment::new();
    let mut set = |name: &str, value: &str, source: Source| {
        let var = EnvVar {
            value: value.to_owned(),
            source,
        };
        environment.insert(name.to_owned(), var);
    };
    for (name, value) in defaults {
        set(name, value, Source::Default);
    }
    for file in files {
        for (name, value) in &file.vars {
            set(name, value, Source::File(file.path.clone()));
        }
    }
    for (name, value) in host {
        if passthrough
            .iter()
            .any(|pattern| glob_matches(pattern, name))
        {
            set(name, value, Source::Host);
        }
    }
    for arg in cli {
        match arg {
            CliEnv::Value(name, value) => set(name, value, Source::Cli),
            CliEnv::FromHost(name) => {
                if let Some((_, value)) = host.iter().find(|(host_name, _)| host_name == name) {
                    set(name, value, Source::Cli);
                }
            }
        }
    }
    environment
}

/// Drops the variables vz sets itself; it sets them after, so they would
/// win anyway, but dropping them says so.
pub fn without_reserved(mut environment: Environment, reserved: &[&str]) -> Environment {
    for name in reserved {
        if let Some(var) = environment.remove(*name) {
            tracing::warn!(
                "env: vz sets {name} itself; ignoring the one from {}",
                var.source
            );
        }
    }
    environment
}

/// Each file read in order. A required one must exist; an optional one is
/// skipped when missing. One tracked by git is refused: its values would be
/// in the repository's history.
fn load_files(files: &[EnvFile], paths: &Paths) -> anyhow::Result<Vec<LoadedFile>> {
    let mut loaded = Vec::new();
    for file in files {
        let written = substitute(&file.path, paths)
            .with_context(|| format!("in env file `{}`", file.path))?;
        let path = resolve_host_path(&written, paths.config_dir, paths.home);
        if !path.exists() {
            ensure!(
                !file.required,
                "env file {} does not exist; mark it `optional` if it may be missing",
                path.display()
            );
            debug!("env file {} is missing; optional, skipped", path.display());
            continue;
        }
        refuse_tracked(&path, paths.repo_root)?;
        let vars = dotenvy::from_path_iter(&path)
            .and_then(|lines| lines.collect::<Result<Vec<_>, _>>())
            .with_context(|| format!("reading env file {}", path.display()))?;
        debug!("env file {}: {} variables", path.display(), vars.len());
        loaded.push(LoadedFile { path, vars });
    }
    Ok(loaded)
}

fn refuse_tracked(path: &Path, repo_root: &Path) -> anyhow::Result<()> {
    let Ok(inside) = path.strip_prefix(repo_root) else {
        return Ok(());
    };
    let tracked = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(inside)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if tracked {
        bail!(
            "env file {} is tracked by git, so its values are in the repository's history; \
             untrack it with `git rm --cached {}` and add it to .gitignore",
            path.display(),
            inside.display()
        );
    }
    Ok(())
}

/// `${repo}` and `${home}`; any other `${…}` is an error. A `$` not followed
/// by `{` stays as it is.
fn substitute(text: &str, paths: &Paths) -> anyhow::Result<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .with_context(|| format!("`{text}` has a `${{` without `}}`"))?;
        match &after[..end] {
            "repo" => out.push_str(&paths.repo_root.to_string_lossy()),
            "home" => out.push_str(&paths.home.to_string_lossy()),
            other => bail!("`${{{other}}}` in `{text}`: vz substitutes ${{repo}} and ${{home}}"),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `*` matches any run of characters, `?` any one.
fn glob_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                backtrack = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match backtrack {
                Some((star, matched)) => {
                    p = star + 1;
                    n = matched + 1;
                    backtrack = Some((star, matched + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn var(value: &str, source: Source) -> EnvVar {
        EnvVar {
            value: value.to_owned(),
            source,
        }
    }

    fn paths() -> Paths<'static> {
        Paths {
            config_dir: Path::new("/home/sally/repos/app"),
            repo_root: Path::new("/home/sally/repos/app"),
            home: Path::new("/home/sally"),
        }
    }

    /// One variable per level, and one defined at every level: the
    /// precedence, read off a single result.
    #[test]
    fn resolve__every_source__later_wins() {
        let defaults = pairs(&[("A", "default"), ("ONLY_DEFAULT", "d")]);
        let files = [
            LoadedFile {
                path: PathBuf::from("/r/.env"),
                vars: pairs(&[("A", "first file"), ("ONLY_FILE", "f")]),
            },
            LoadedFile {
                path: PathBuf::from("/r/.env.local"),
                vars: pairs(&[("A", "second file")]),
            },
        ];
        let passthrough = ["A".to_owned(), "HOST_*".to_owned()];
        let host = pairs(&[
            ("A", "host"),
            ("HOST_ONE", "h"),
            ("NOT_PASSED", "x"),
            ("B", "b"),
        ]);
        let cli = [
            CliEnv::Value("A".to_owned(), "cli".to_owned()),
            CliEnv::FromHost("B".to_owned()),
            CliEnv::FromHost("UNSET".to_owned()),
        ];

        let environment = resolve(&defaults, &files, &passthrough, &host, &cli);

        let expected = Environment::from([
            ("A".to_owned(), var("cli", Source::Cli)),
            ("B".to_owned(), var("b", Source::Cli)),
            ("HOST_ONE".to_owned(), var("h", Source::Host)),
            ("ONLY_DEFAULT".to_owned(), var("d", Source::Default)),
            (
                "ONLY_FILE".to_owned(),
                var("f", Source::File(PathBuf::from("/r/.env"))),
            ),
        ]);
        assert_eq!(environment, expected);
    }

    #[test]
    fn resolve__files__the_later_file_wins() {
        let files = [
            LoadedFile {
                path: PathBuf::from("/r/.env"),
                vars: pairs(&[("A", "first")]),
            },
            LoadedFile {
                path: PathBuf::from("/r/.env.local"),
                vars: pairs(&[("A", "second")]),
            },
        ];

        let environment = resolve(&[], &files, &[], &[], &[]);

        let expected = var("second", Source::File(PathBuf::from("/r/.env.local")));
        assert_eq!(environment["A"], expected);
    }

    #[test]
    fn without_reserved__vz_names__dropped() {
        let environment = Environment::from([
            ("HOME".to_owned(), var("/elsewhere", Source::Default)),
            ("KEEP".to_owned(), var("k", Source::Default)),
        ]);

        let kept = without_reserved(environment, &["HOME", "VZ_UID"]);

        assert_eq!(kept.keys().collect::<Vec<_>>(), ["KEEP"]);
    }

    #[test]
    fn substitute__cases() {
        let cases = [
            ("plain", "plain"),
            ("${repo}/data", "/home/sally/repos/app/data"),
            ("${home}/.cache", "/home/sally/.cache"),
            ("cost: $5 and $HOME", "cost: $5 and $HOME"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                substitute(text, &paths()).unwrap(),
                expected,
                "text: {text}"
            );
        }
    }

    #[test]
    fn substitute__unknown_or_unclosed__is_refused_naming_it() {
        for (text, expected) in [
            ("${profile}", "${profile}"),
            ("${env:X}", "${env:X}"),
            ("${repo", "without"),
        ] {
            let error = substitute(text, &paths()).unwrap_err().to_string();

            assert!(error.contains(expected), "{text}: {error}");
        }
    }

    #[test]
    fn glob_matches__cases() {
        let cases = [
            ("GH_TOKEN", "GH_TOKEN", true),
            ("GH_TOKEN", "GH_TOKEN_X", false),
            ("FMP_*", "FMP_API_KEY", true),
            ("FMP_*", "FMP_", true),
            ("FMP_*", "XFMP_A", false),
            ("*_TOKEN", "GH_TOKEN", true),
            ("A?C", "ABC", true),
            ("A?C", "AC", false),
            ("*", "ANY", true),
            ("A*B*C", "AXXBYYC", true),
            ("A*B*C", "AXXBYY", false),
        ];
        for (pattern, name, expected) in cases {
            assert_eq!(glob_matches(pattern, name), expected, "{pattern} ~ {name}");
        }
    }

    #[test]
    fn cli_env_parse__forms() {
        assert_eq!(
            CliEnv::parse("A=1=2").unwrap(),
            CliEnv::Value("A".to_owned(), "1=2".to_owned())
        );
        assert_eq!(
            CliEnv::parse("A=").unwrap(),
            CliEnv::Value("A".to_owned(), String::new())
        );
        assert_eq!(
            CliEnv::parse("GH_TOKEN").unwrap(),
            CliEnv::FromHost("GH_TOKEN".to_owned())
        );
        assert!(CliEnv::parse("MY-VAR=x").is_err());
    }

    #[test]
    fn load_files__required_missing__is_refused_optional_missing__skipped() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: dir.path(),
            repo_root: dir.path(),
            home: dir.path(),
        };
        let required = EnvFile {
            path: ".env".to_owned(),
            required: true,
        };
        let optional = EnvFile {
            path: ".env".to_owned(),
            required: false,
        };

        let refused = load_files(&[required], &paths).unwrap_err().to_string();
        let skipped = load_files(&[optional], &paths).unwrap();

        assert!(refused.contains("does not exist"), "{refused}");
        assert_eq!(skipped, vec![]);
    }

    #[test]
    fn load_files__dotenv_syntax__read_in_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "# a comment\nA=1\nexport B=\"two words\"\nC='single'\n",
        )
        .unwrap();
        let paths = Paths {
            config_dir: dir.path(),
            repo_root: Path::new("/elsewhere"),
            home: dir.path(),
        };
        let file = EnvFile {
            path: ".env".to_owned(),
            required: true,
        };

        let loaded = load_files(&[file], &paths).unwrap();

        assert_eq!(
            loaded[0].vars,
            pairs(&[("A", "1"), ("B", "two words"), ("C", "single")])
        );
    }

    #[test]
    fn load_files__tracked_by_git__is_refused_naming_the_fix() {
        let repo = tempfile::tempdir().unwrap();
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
        std::fs::write(repo.path().join(".env"), "SECRET=1\n").unwrap();
        git(&["add", ".env"]);
        let paths = Paths {
            config_dir: repo.path(),
            repo_root: repo.path(),
            home: repo.path(),
        };
        let file = EnvFile {
            path: ".env".to_owned(),
            required: true,
        };

        let error = load_files(&[file], &paths).unwrap_err().to_string();

        assert!(error.contains("git rm --cached .env"), "{error}");
    }
}
