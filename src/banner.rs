//! `banner:`: the viz-shell banner above an interactive shell, in the
//! manner of fastfetch: the art on top, what the shell is below it.

use std::path::{Path, PathBuf};

use anstyle::{AnsiColor, Style};

use crate::config::{Link, banner_line, tilde};
use crate::constants::DEFAULT_CONFIG;
use crate::mounts::HostMount;

/// The built-in art, `banner: true`: `figlet viz-shell`, its lines joined by
/// `\n`, no trailing newline.
pub const ART_TEXT: &str = concat!(
    r"       _              _          _ _",
    "\n",
    r"__   _(_)____     ___| |__   ___| | |",
    "\n",
    r"\ \ / / |_  /____/ __| '_ \ / _ \ | |",
    "\n",
    r" \ V /| |/ /_____\__ \ | | |  __/ | |",
    "\n",
    r"  \_/ |_/___|    |___/_| |_|\___|_|_|",
);

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
    /// The configuration chain, one link per step, in fold order.
    pub chain: &'a [Link],
    /// The files of the chain, each once, in fold order: whose mounts are counted.
    pub config_files: &'a [PathBuf],
    pub config: Option<&'a str>,
    pub image: &'a str,
    /// The images it is built on, the nearest first.
    pub image_bases: &'a [&'a str],
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
    let per_file: Vec<(usize, String)> = session
        .config_files
        .iter()
        .filter_map(|file| {
            let count = session.mounts.iter().filter(|m| m.file == *file).count();
            let name = file.file_name()?.to_string_lossy().into_owned();
            (count > 0).then_some((count, name))
        })
        .collect();
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
        ("Chain", or_none(banner_line(session.chain))),
        (
            "Config",
            session.config.unwrap_or(DEFAULT_CONFIG).to_owned(),
        ),
        (
            "Image",
            match session.image_bases {
                [] => session.image.to_owned(),
                bases => format!("{} (on {})", session.image, bases.join(", ")),
            },
        ),
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

/// The art and a blank line, unless the art is empty, then the title, a
/// rule, and the facts; styled.
/// Print it through `anstream`, which drops the styles where they don't
/// belong: off a terminal, or with `NO_COLOR`.
pub fn render(art: &str, title: &str, facts: &[(&str, String)]) -> String {
    let paint = |style: &Style, text: &str| format!("{style}{text}{style:#}");
    let art: Vec<String> = match art.is_empty() {
        true => Vec::new(),
        false => art
            .lines()
            .map(|line| paint(&ART_STYLE, line))
            .chain([String::new()])
            .collect(),
    };
    let facts = facts
        .iter()
        .map(|(key, value)| format!("{}: {value}", paint(&KEY_STYLE, key)));
    let lines: Vec<String> = art
        .into_iter()
        .chain([paint(&KEY_STYLE, title), "-".repeat(title.chars().count())])
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

    fn session<'a>(mounts: &'a [HostMount], files: &'a [PathBuf]) -> Session<'a> {
        Session {
            user: "sally",
            container: "vz-0-app",
            attached: false,
            persistent: false,
            home: Path::new("/home/sally"),
            repo_root: Path::new("/home/sally/repos/app"),
            branch: Some("main"),
            chain: &[],
            config_files: files,
            config: Some("trusted"),
            image: "vz-app:0123456789abcdef",
            image_bases: &[],
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

    const LIBRARY_BASE: &str = "/home/sally/.config/viz-shell/vz-debian-trixie.vz.yml";
    const REPOSITORY: &str = "/home/sally/repos/app/app.vz.yml";
    const SALLY: &str = "/home/sally/repos/app/sally.vz.yml";

    fn mount(target: &str, file: &str) -> HostMount {
        HostMount {
            source: PathBuf::from(target),
            target: PathBuf::from(target),
            read_only: false,
            point_in_state: None,
            file: PathBuf::from(file),
        }
    }

    fn files(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    /// The value of the fact `key`, if the banner has that line.
    fn value<'f>(facts: &'f [(&str, String)], key: &str) -> Option<&'f str> {
        facts
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }

    fn chain() -> Vec<Link> {
        [
            "vz-debian-trixie@~/.config/viz-shell/vz-debian-trixie.vz.yml",
            "trusted@app.vz.yml",
        ]
        .map(Link::parse)
        .to_vec()
    }

    /// The whole list on purpose: the lines and their order are the banner's
    /// layout, the requirement itself.
    #[test]
    fn facts__session__one_line_each_in_order() {
        let mounts = [
            mount("/home/sally/repos", LIBRARY_BASE),
            mount("/home/sally/.ssh", REPOSITORY),
            mount("/etc/hosts", REPOSITORY),
        ];
        let files = files(&[LIBRARY_BASE, REPOSITORY]);
        let chain = chain();
        let session = Session {
            chain: &chain,
            ..session(&mounts, &files)
        };

        let facts = facts(&session);

        let expected = [
            ("Version", VERSION),
            ("Session", "new, ephemeral"),
            ("Repo", "~/repos/app"),
            ("Branch", "main"),
            ("Chain", "vz-debian-trixie (library) → trusted"),
            ("Config", "trusted"),
            ("Image", "vz-app:0123456789abcdef"),
            ("Shell", "fish"),
            ("Sudo", "yes"),
            ("Docker", "/run/user/1000/docker.sock"),
            ("Network", "bridge"),
            ("Mounts", "3 (1 vz-debian-trixie.vz.yml, 2 app.vz.yml)"),
            ("State", "2 paths in ~/repos/app/.vz_state"),
            ("Env", "1 variable"),
            ("Hooks", "2 create, 1 attach"),
        ];
        let facts: Vec<(&str, &str)> = facts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(facts, expected);
    }

    /// A session that sets nothing it can leave unset.
    fn nothing_set() -> Session<'static> {
        Session {
            branch: None,
            config: None,
            shell: None,
            sudo: false,
            docker: None,
            state_paths: 0,
            env_vars: 0,
            create_hooks: 0,
            attach_hooks: 0,
            ..session(&[], &[])
        }
    }

    #[test]
    fn facts__no_branch__no_branch_line() {
        let facts = facts(&nothing_set());

        assert_eq!(value(&facts, "Branch"), None);
    }

    #[test]
    fn facts__nothing_set__says_so() {
        let facts = facts(&nothing_set());

        let expected = [
            ("Chain", "none"),
            ("Config", "default"),
            ("Shell", "bash (else sh)"),
            ("Sudo", "no"),
            ("Docker", "none"),
            ("Mounts", "none"),
            ("State", "none"),
            ("Env", "none"),
            ("Hooks", "none"),
        ];
        for (key, expected) in expected {
            assert_eq!(value(&facts, key), Some(expected), "{key}");
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

            let value = value(&facts, "Session").unwrap();
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

            let value = value(&facts, "Hooks").unwrap();
            assert_eq!(
                value, expected,
                "create {create_hooks}, attach {attach_hooks}"
            );
        }
    }

    #[test]
    fn facts__image__top_then_its_bases_nearest_first() {
        let cases: [(&[&str], &str); 3] = [
            (&[], "vz-app:3f9c2a1b"),
            (
                &["debian:stable-slim"],
                "vz-app:3f9c2a1b (on debian:stable-slim)",
            ),
            (
                &["vz-tools:9a1c0d2e", "debian:stable-slim"],
                "vz-app:3f9c2a1b (on vz-tools:9a1c0d2e, debian:stable-slim)",
            ),
        ];
        for (image_bases, expected) in cases {
            let session = Session {
                image: "vz-app:3f9c2a1b",
                image_bases,
                ..session(&[], &[])
            };

            let facts = facts(&session);

            let value = value(&facts, "Image").unwrap();
            assert_eq!(value, expected, "{image_bases:?}");
        }
    }

    #[test]
    fn facts__state_paths__none_one_or_many_in_the_state_dir() {
        let cases = [
            (0, "none"),
            (1, "1 path in ~/repos/app/.vz_state"),
            (2, "2 paths in ~/repos/app/.vz_state"),
        ];
        for (state_paths, expected) in cases {
            let session = Session {
                state_paths,
                ..session(&[], &[])
            };

            let facts = facts(&session);

            let value = value(&facts, "State").unwrap();
            assert_eq!(value, expected, "{state_paths}");
        }
    }

    #[test]
    fn facts__env_vars__none_one_or_many_counted() {
        let cases = [(0, "none"), (1, "1 variable"), (3, "3 variables")];
        for (env_vars, expected) in cases {
            let session = Session {
                env_vars,
                ..session(&[], &[])
            };

            let facts = facts(&session);

            let value = value(&facts, "Env").unwrap();
            assert_eq!(value, expected, "{env_vars}");
        }
    }

    #[test]
    fn facts__host_network__host() {
        let session = Session {
            host_network: true,
            ..session(&[], &[])
        };

        let facts = facts(&session);

        let value = value(&facts, "Network").unwrap();
        assert_eq!(value, "host");
    }

    #[test]
    fn facts__mounts__counted_per_file_by_its_name_in_chain_order() {
        let cases: [(&str, &[&str], &[&str], &str); 5] = [
            (
                "one from the library",
                &[LIBRARY_BASE],
                &[LIBRARY_BASE],
                "1 (vz-debian-trixie.vz.yml)",
            ),
            (
                "one from the repository",
                &[REPOSITORY],
                &[REPOSITORY],
                "1 (app.vz.yml)",
            ),
            (
                "two from one of two files",
                &[LIBRARY_BASE, REPOSITORY],
                &[REPOSITORY, REPOSITORY],
                "2 (app.vz.yml)",
            ),
            (
                "a file without mounts left out",
                &[REPOSITORY, SALLY],
                &[SALLY],
                "1 (sally.vz.yml)",
            ),
            (
                "three files, in chain order not mount order",
                &[LIBRARY_BASE, REPOSITORY, SALLY],
                &[SALLY, LIBRARY_BASE, REPOSITORY],
                "3 (1 vz-debian-trixie.vz.yml, 1 app.vz.yml, 1 sally.vz.yml)",
            ),
        ];
        for (case, chain_files, mounted_by, expected) in cases {
            let files = files(chain_files);
            let mounts: Vec<HostMount> = mounted_by
                .iter()
                .map(|file| mount("/home/sally/anywhere", file))
                .collect();

            let facts = facts(&session(&mounts, &files));

            let value = value(&facts, "Mounts").unwrap();
            assert_eq!(value, expected, "{case}");
        }
    }

    #[test]
    fn title__session__user_at_the_container() {
        let title = title(&session(&[], &[]));

        assert_eq!(title, "sally@vz-0-app");
    }

    /// The banner of `art` over Sally's title and one fact, unstyled, by line.
    fn rendered(art: &str) -> Vec<String> {
        let facts = [("Repo", "~/repos/app".to_owned())];
        let banner = render(art, "sally@app", &facts);
        anstream::adapter::strip_str(&banner)
            .to_string()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn render__art__its_lines_a_blank_line_title_facts_and_a_blank_line() {
        let cases: [(&str, &str, &[&str]); 3] = [
            (
                "two lines of its own",
                "== app ==\n  v1",
                &[
                    "== app ==",
                    "  v1",
                    "",
                    "sally@app",
                    "---------",
                    "Repo: ~/repos/app",
                    "",
                ],
            ),
            (
                "one line of its own",
                "== app ==",
                &[
                    "== app ==",
                    "",
                    "sally@app",
                    "---------",
                    "Repo: ~/repos/app",
                    "",
                ],
            ),
            (
                "empty: no art, just the facts",
                "",
                &["sally@app", "---------", "Repo: ~/repos/app", ""],
            ),
        ];
        for (case, art, expected) in cases {
            let lines = rendered(art);

            assert_eq!(lines, expected, "{case}");
        }
    }

    #[test]
    fn render__built_in_art__its_five_lines_first() {
        let lines = rendered(ART_TEXT);

        assert_eq!(lines[..5], ART_TEXT.lines().collect::<Vec<_>>()[..]);
    }

    #[test]
    fn render__any_art__ends_with_a_blank_line() {
        let facts = [("Repo", "~/repos/app".to_owned())];

        let banner = render("== app ==", "sally@app", &facts);

        assert!(banner.ends_with("\n\n"), "{banner}");
    }

    // The styles are named by their constants: which escape codes a style
    // writes is anstyle's business, not the banner's.
    #[test]
    fn render__color__keys_colored() {
        let facts = [("Repo", "~/repos/app".to_owned())];

        let banner = render(ART_TEXT, "sally@app", &facts);

        assert!(
            banner.contains(&format!("{KEY_STYLE}Repo{KEY_STYLE:#}: ~/repos/app")),
            "{banner}"
        );
    }

    #[test]
    fn render__color__art_colored() {
        let facts = [("Repo", "~/repos/app".to_owned())];

        let banner = render(ART_TEXT, "sally@app", &facts);

        assert!(banner.starts_with(&ART_STYLE.to_string()), "{banner}");
    }
}
