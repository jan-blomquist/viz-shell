//! The environment inside the container: each variable's value, and where it
//! came from. Sources, later wins: `env.defaults`, `env.files` in order,
//! `env.passthrough`, then `vz --env`. Values are read on the host and reach
//! the container by name only: never on a command line, never in a log.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use tracing::{debug, warn};

use crate::config::{EffectiveEnv, EnvFile, is_env_name, resolve_host_path, substitute};

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
            let value = substitute(value, paths.repo_root, paths.home)
                .with_context(|| format!("in env default `{name}`"))?;
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
/// skipped when missing. One tracked by git is loaded with a warning: its
/// values are in the repository's history.
fn load_files(files: &[EnvFile], paths: &Paths) -> anyhow::Result<Vec<LoadedFile>> {
    let mut loaded = Vec::new();
    for file in files {
        let written = substitute(&file.path, paths.repo_root, paths.home)
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
        if path.starts_with(paths.repo_root) && crate::repo::is_tracked(&path) {
            warn!(
                "env file {} is tracked by git: its values are in the repository's history",
                path.display()
            );
        }
        let vars = dotenvy::from_path_iter(&path)
            .and_then(|lines| lines.collect::<Result<Vec<_>, _>>())
            .with_context(|| format!("reading env file {}", path.display()))?;
        debug!("env file {}: {} variables", path.display(), vars.len());
        loaded.push(LoadedFile { path, vars });
    }
    Ok(loaded)
}

/// `*` matches any run of characters, `?` any one. The configuration
/// allows no other glob syntax, so every pattern compiles.
fn glob_matches(pattern: &str, name: &str) -> bool {
    glob::Pattern::new(pattern).is_ok_and(|pattern| pattern.matches(name))
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};

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

        let names: Vec<&String> = kept.keys().collect();
        assert_eq!(names, ["KEEP"]);
    }

    #[test]
    fn glob_matches__a_name_the_pattern_covers__matches() {
        let cases = [
            ("GH_TOKEN", "GH_TOKEN"),
            ("FMP_*", "FMP_API_KEY"),
            ("FMP_*", "FMP_"),
            ("*_TOKEN", "GH_TOKEN"),
            ("A?C", "ABC"),
            ("*", "ANY"),
            ("A*B*C", "AXXBYYC"),
        ];
        for (pattern, name) in cases {
            let matches = glob_matches(pattern, name);

            assert!(matches, "{pattern} ~ {name}");
        }
    }

    #[test]
    fn glob_matches__any_other_name__no_match() {
        let cases = [
            ("GH_TOKEN", "GH_TOKEN_X"),
            ("FMP_*", "XFMP_A"),
            ("A?C", "AC"),
            ("A*B*C", "AXXBYY"),
        ];
        for (pattern, name) in cases {
            let matches = glob_matches(pattern, name);

            assert!(!matches, "{pattern} ~ {name}");
        }
    }

    #[test]
    fn cli_env_parse__name_equals_value__a_value_split_at_the_first_equals() {
        let cases = [("A=1=2", "A", "1=2"), ("A=", "A", "")];
        for (arg, name, value) in cases {
            let parsed = CliEnv::parse(arg).unwrap();

            assert_eq!(
                parsed,
                CliEnv::Value(name.to_owned(), value.to_owned()),
                "{arg}"
            );
        }
    }

    #[test]
    fn cli_env_parse__a_bare_name__copied_from_the_host() {
        let parsed = CliEnv::parse("GH_TOKEN").unwrap();

        assert_eq!(parsed, CliEnv::FromHost("GH_TOKEN".to_owned()));
    }

    #[test]
    fn cli_env_parse__not_a_variable_name__refused_naming_it() {
        let result = CliEnv::parse("MY-VAR=x");

        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("--env MY-VAR=x: `MY-VAR` is not a variable name"),
            "{message}"
        );
    }

    /// `.env`, `required` or not, in an empty folder.
    fn missing_env_file(required: bool) -> (tempfile::TempDir, EnvFile) {
        let dir = tempfile::tempdir().unwrap();
        let file = EnvFile {
            path: ".env".to_owned(),
            required,
        };
        (dir, file)
    }

    fn paths_in(dir: &Path) -> Paths<'_> {
        Paths {
            config_dir: dir,
            repo_root: dir,
            home: dir,
        }
    }

    #[test]
    fn load_files__required_missing__is_refused() {
        let (dir, required) = missing_env_file(true);

        let result = load_files(&[required], &paths_in(dir.path()));

        let message = format!("{:#}", result.unwrap_err());
        assert!(message.contains(".env does not exist"), "{message}");
    }

    #[test]
    fn load_files__optional_missing__skipped() {
        let (dir, optional) = missing_env_file(false);

        let loaded = load_files(&[optional], &paths_in(dir.path())).unwrap();

        assert_eq!(loaded, []);
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

    /// A git repository whose `.env`, `SECRET=1`, git tracks: the files
    /// read from it, and what was logged while reading.
    fn load_tracked_env_file() -> (Vec<LoadedFile>, String) {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        std::fs::write(repo.path().join(".env"), "SECRET=1\n").unwrap();
        git(repo.path(), &["add", ".env"]);
        let file = EnvFile {
            path: ".env".to_owned(),
            required: true,
        };
        let log = Log::default();
        let loaded = tracing::subscriber::with_default(log.subscriber(), || {
            load_files(&[file], &paths_in(repo.path())).unwrap()
        });
        (loaded, log.text())
    }

    #[test]
    fn load_files__tracked_by_git__still_loaded() {
        let (loaded, _) = load_tracked_env_file();

        assert_eq!(loaded[0].vars, pairs(&[("SECRET", "1")]));
    }

    #[test]
    fn load_files__tracked_by_git__a_warning_logged() {
        let (_, log) = load_tracked_env_file();

        assert!(log.contains("is tracked by git"), "{log}");
    }

    /// What a subscriber logged, as text.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl Log {
        fn subscriber(&self) -> impl tracing::Subscriber + use<> {
            let log = self.clone();
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(move || log.clone())
                .finish()
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl std::io::Write for Log {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
