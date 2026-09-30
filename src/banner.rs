//! `banner: true`: the viz-shell banner above an interactive shell, in the
//! manner of fastfetch: the art on top, what the shell is below it.

use std::path::Path;

use anstyle::{AnsiColor, Style};

use crate::config::{ConfigFile, tilde};
use crate::mounts::HostMount;

/// `figlet viz-shell`.
const ART: [&str; 5] = [
    r"       _              _          _ _",
    r"__   _(_)____     ___| |__   ___| | |",
    r"\ \ / / |_  /____/ __| '_ \ / _ \ | |",
    r" \ V /| |/ /_____\__ \ | | |  __/ | |",
    r"  \_/ |_/___|    |___/_| |_|\___|_|_|",
];

/// This build: the version in Cargo.toml.
const VERSION: &str = env!("CARGO_PKG_VERSION");

const ART_STYLE: Style = AnsiColor::Cyan.on_default();
const KEY_STYLE: Style = AnsiColor::Cyan.on_default().bold();

/// What the banner reports: the session vz is about to start.
pub struct Session<'a> {
    pub user: &'a str,
    /// The container's name, which is its hostname too.
    pub container: &'a str,
    /// Joining a container that already exists, rather than a new one.
    pub attached: bool,
    /// The container outlives the shell.
    pub persistent: bool,
    pub home: &'a Path,
    pub repo_root: &'a Path,
    pub branch: Option<&'a str>,
    /// The configuration files read, each with the file it is: global,
    /// repository, local, in that order.
    pub config_files: &'a [(ConfigFile, &'a Path)],
    pub profile: Option<&'a str>,
    pub image: &'a str,
    pub shell: Option<&'a str>,
    pub sudo: bool,
    /// The host's docker socket, when shared.
    pub docker: Option<&'a Path>,
    pub host_network: bool,
    pub mounts: &'a [HostMount],
    pub state_paths: usize,
    pub state_dir: &'a Path,
    /// How many variables the configured environment sets; never their names.
    pub env_vars: usize,
    pub create_hooks: usize,
    pub attach_hooks: usize,
}

/// The title, `user@container`: the prompt's `user@hostname`.
pub fn title(session: &Session) -> String {
    format!("{}@{}", session.user, session.container)
}

/// One `(key, value)` per line under the title.
pub fn facts(session: &Session) -> Vec<(&'static str, String)> {
    let home = session.home;
    let or_none = |text: String| match text.is_empty() {
        true => "none".to_owned(),
        false => text,
    };
    let file_names: Vec<(ConfigFile, String)> = session
        .config_files
        .iter()
        .filter_map(|(file, path)| Some((*file, path.file_name()?.to_string_lossy().into_owned())))
        .collect();
    let per_file: Vec<(usize, &str)> = file_names
        .iter()
        .filter_map(|(file, name)| {
            let count = session.mounts.iter().filter(|m| m.file == *file).count();
            (count > 0).then_some((count, name.as_str()))
        })
        .collect();
    let file_names: Vec<&str> = file_names.iter().map(|(_, name)| name.as_str()).collect();
    let mounts = match per_file.as_slice() {
        [] => "none".to_owned(),
        [(count, name)] => format!("{count} ({name})"),
        _ => {
            let counts: Vec<String> = per_file
                .iter()
                .map(|(count, name)| format!("{count} {name}"))
                .collect();
            format!("{} ({})", session.mounts.len(), counts.join(", "))
        }
    };
    let state = match session.state_paths {
        0 => "none".to_owned(),
        1 => format!("1 path in {}", tilde(session.state_dir, home)),
        paths => format!("{paths} paths in {}", tilde(session.state_dir, home)),
    };
    let env = match session.env_vars {
        0 => "none".to_owned(),
        1 => "1 variable".to_owned(),
        vars => format!("{vars} variables"),
    };
    let hook_counts: Vec<String> = [
        (session.create_hooks, "create"),
        (session.attach_hooks, "attach"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, kind)| format!("{count} {kind}"))
    .collect();
    let how = if session.attached { "attached" } else { "new" };
    let lifetime = if session.persistent {
        "persistent"
    } else {
        "ephemeral"
    };
    let mut facts = vec![
        ("Version", VERSION.to_owned()),
        ("Session", format!("{how}, {lifetime}")),
        ("Repo", tilde(session.repo_root, home)),
    ];
    if let Some(branch) = session.branch {
        facts.push(("Branch", branch.to_owned()));
    }
    facts.extend([
        ("Config", or_none(file_names.join(", "))),
        ("Profile", session.profile.unwrap_or("none").to_owned()),
        ("Image", session.image.to_owned()),
        (
            "Shell",
            session.shell.unwrap_or("bash (else sh)").to_owned(),
        ),
        (
            "Sudo",
            match session.sudo {
                true => "yes".to_owned(),
                false => "no".to_owned(),
            },
        ),
        (
            "Docker",
            session
                .docker
                .map_or("none".to_owned(), |socket| socket.display().to_string()),
        ),
        (
            "Network",
            match session.host_network {
                true => "host".to_owned(),
                false => "bridge".to_owned(),
            },
        ),
        ("Mounts", mounts),
        ("State", state),
        ("Env", env),
        ("Hooks", or_none(hook_counts.join(", "))),
    ]);
    facts
}

/// The art, a blank line, then the title, a rule, and the facts; styled.
/// Print it through `anstream`, which drops the styles where they don't
/// belong: off a terminal, or with `NO_COLOR`.
pub fn render(title: &str, facts: &[(&str, String)]) -> String {
    let paint = |style: &Style, text: &str| format!("{style}{text}{style:#}");
    let art = ART.iter().map(|line| paint(&ART_STYLE, line));
    let facts = facts
        .iter()
        .map(|(key, value)| format!("{}: {value}", paint(&KEY_STYLE, key)));
    let lines: Vec<String> = art
        .chain([
            String::new(),
            paint(&KEY_STYLE, title),
            "-".repeat(title.chars().count()),
        ])
        .chain(facts)
        .collect();
    // A blank line between the banner and the prompt.
    format!("{}\n\n", lines.join("\n"))
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn session<'a>(mounts: &'a [HostMount], files: &'a [(ConfigFile, &'a Path)]) -> Session<'a> {
        Session {
            user: "sally",
            container: "vz-0-app",
            attached: false,
            persistent: false,
            home: Path::new("/home/sally"),
            repo_root: Path::new("/home/sally/repos/app"),
            branch: Some("main"),
            config_files: files,
            profile: Some("trusted"),
            image: "vz-app:0123456789abcdef",
            shell: Some("fish"),
            sudo: true,
            docker: Some(Path::new("/run/user/1000/docker.sock")),
            host_network: false,
            mounts,
            state_paths: 2,
            state_dir: Path::new("/home/sally/repos/app/.vz_state"),
            env_vars: 1,
            create_hooks: 2,
            attach_hooks: 1,
        }
    }

    fn mount(target: &str, file: ConfigFile) -> HostMount {
        HostMount {
            source: PathBuf::from(target),
            target: PathBuf::from(target),
            read_only: false,
            point_in_state: None,
            file,
        }
    }

    /// A configuration file of the kind, under a typical name.
    fn file(kind: ConfigFile) -> (ConfigFile, &'static Path) {
        let path = match kind {
            ConfigFile::Global => "/home/sally/.config/viz-shell/global.yml",
            ConfigFile::Repository => "/home/sally/repos/app/viz-shell.yml",
            ConfigFile::Local => "/home/sally/repos/app/viz-shell.local.yml",
        };
        (kind, Path::new(path))
    }

    fn both_files() -> [(ConfigFile, &'static Path); 2] {
        [file(ConfigFile::Global), file(ConfigFile::Repository)]
    }

    #[test]
    fn facts__session__one_line_each_in_order() {
        let mounts = [
            mount("/home/sally/repos", ConfigFile::Global),
            mount("/home/sally/.ssh", ConfigFile::Repository),
            mount("/etc/hosts", ConfigFile::Repository),
        ];
        let files = both_files();

        let facts = facts(&session(&mounts, &files));

        let expected = [
            ("Version", VERSION),
            ("Session", "new, ephemeral"),
            ("Repo", "~/repos/app"),
            ("Branch", "main"),
            ("Config", "global.yml, viz-shell.yml"),
            ("Profile", "trusted"),
            ("Image", "vz-app:0123456789abcdef"),
            ("Shell", "fish"),
            ("Sudo", "yes"),
            ("Docker", "/run/user/1000/docker.sock"),
            ("Network", "bridge"),
            ("Mounts", "3 (1 global.yml, 2 viz-shell.yml)"),
            ("State", "2 paths in ~/repos/app/.vz_state"),
            ("Env", "1 variable"),
            ("Hooks", "2 create, 1 attach"),
        ];
        let facts: Vec<(&str, &str)> = facts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(facts, expected);
    }

    #[test]
    fn facts__nothing_set__says_so() {
        let session = Session {
            branch: None,
            profile: None,
            shell: None,
            sudo: false,
            docker: None,
            state_paths: 0,
            env_vars: 0,
            create_hooks: 0,
            attach_hooks: 0,
            ..session(&[], &[])
        };

        let facts = facts(&session);

        let value = |key: &str| {
            facts
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(value("Branch"), None);
        let expected = [
            ("Config", "none"),
            ("Profile", "none"),
            ("Shell", "bash (else sh)"),
            ("Sudo", "no"),
            ("Docker", "none"),
            ("Mounts", "none"),
            ("State", "none"),
            ("Env", "none"),
            ("Hooks", "none"),
        ];
        for (key, expected) in expected {
            assert_eq!(value(key), Some(expected), "{key}");
        }
    }

    #[test]
    fn facts__session_kinds__new_or_attached_then_its_lifetime() {
        let cases = [
            (false, false, "new, ephemeral"),
            (false, true, "new, persistent"),
            (true, false, "attached, ephemeral"),
            (true, true, "attached, persistent"),
        ];
        for (attached, persistent, expected) in cases {
            let session = Session {
                attached,
                persistent,
                ..session(&[], &[])
            };

            let facts = facts(&session);

            let (_, value) = facts.iter().find(|(key, _)| *key == "Session").unwrap();
            assert_eq!(
                value, expected,
                "attached {attached}, persistent {persistent}"
            );
        }
    }

    #[test]
    fn facts__hooks__counts_of_the_kinds_set() {
        let cases = [
            (0, 0, "none"),
            (2, 0, "2 create"),
            (0, 1, "1 attach"),
            (2, 1, "2 create, 1 attach"),
        ];
        for (create_hooks, attach_hooks, expected) in cases {
            let session = Session {
                create_hooks,
                attach_hooks,
                ..session(&[], &[])
            };

            let facts = facts(&session);

            let (_, value) = facts.iter().find(|(key, _)| *key == "Hooks").unwrap();
            assert_eq!(
                value, expected,
                "create {create_hooks}, attach {attach_hooks}"
            );
        }
    }

    #[test]
    fn facts__host_network__host() {
        let session = Session {
            host_network: true,
            ..session(&[], &[])
        };

        let facts = facts(&session);

        let (_, value) = facts.iter().find(|(key, _)| *key == "Network").unwrap();
        assert_eq!(value, "host");
    }

    #[test]
    fn facts__config_and_mounts__named_by_the_file_that_set_them() {
        use ConfigFile::{Global, Local, Repository};
        let cases: [(&[ConfigFile], &[ConfigFile], &str, &str); 6] = [
            (&[Global], &[Global], "global.yml", "1 (global.yml)"),
            (
                &[Repository],
                &[Repository],
                "viz-shell.yml",
                "1 (viz-shell.yml)",
            ),
            (
                &[Global, Repository],
                &[Repository, Repository],
                "global.yml, viz-shell.yml",
                "2 (viz-shell.yml)",
            ),
            (
                &[Repository, Local],
                &[Local],
                "viz-shell.yml, viz-shell.local.yml",
                "1 (viz-shell.local.yml)",
            ),
            (
                &[Global, Repository, Local],
                &[Global, Local],
                "global.yml, viz-shell.yml, viz-shell.local.yml",
                "2 (1 global.yml, 1 viz-shell.local.yml)",
            ),
            (
                &[Global, Repository, Local],
                &[Global, Repository, Local],
                "global.yml, viz-shell.yml, viz-shell.local.yml",
                "3 (1 global.yml, 1 viz-shell.yml, 1 viz-shell.local.yml)",
            ),
        ];
        for (files, mounted_by, expected_config, expected_mounts) in cases {
            let files: Vec<(ConfigFile, &Path)> = files.iter().copied().map(file).collect();
            let mounts: Vec<HostMount> = mounted_by
                .iter()
                .enumerate()
                .map(|(i, kind)| mount(&format!("/home/sally/m{i}"), *kind))
                .collect();

            let facts = facts(&session(&mounts, &files));

            let value = |key: &str| {
                let (_, value) = facts.iter().find(|(k, _)| *k == key).unwrap();
                value.as_str()
            };
            assert_eq!(value("Config"), expected_config);
            assert_eq!(value("Mounts"), expected_mounts, "{expected_config}");
        }
    }

    #[test]
    fn title__session__user_at_the_container() {
        assert_eq!(title(&session(&[], &[])), "sally@vz-0-app");
    }

    #[test]
    fn render__facts__below_the_art_title_first() {
        let facts = [("Repo", "~/repos/app".to_owned())];

        let banner = anstream::adapter::strip_str(&render("sally@app", &facts)).to_string();

        let lines: Vec<&str> = banner.lines().collect();
        assert_eq!(lines[..ART.len()], ART);
        assert_eq!(
            lines[ART.len()..ART.len() + 4],
            ["", "sally@app", "---------", "Repo: ~/repos/app"]
        );
        assert!(banner.ends_with("\n\n"), "{banner}");
        assert!(!banner.contains('\x1b'), "{banner}");
    }

    #[test]
    fn render__color__art_and_keys_colored() {
        let facts = [("Repo", "~/repos/app".to_owned())];

        let banner = render("sally@app", &facts);

        assert!(banner.contains(&format!("{KEY_STYLE}Repo{KEY_STYLE:#}: ~/repos/app")));
        assert!(banner.starts_with(&ART_STYLE.to_string()), "{banner}");
    }
}
