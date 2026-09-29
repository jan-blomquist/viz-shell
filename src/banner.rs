//! `banner: true`: the viz-shell banner above an interactive shell, in the
//! manner of fastfetch: the art on the left, what the shell is on the right.

use std::path::Path;

use crate::config::tilde;
use crate::mounts::HostMount;

const ART: &str = include_str!("../templates/banner.txt");

/// Between the art and the facts.
const GAP: &str = "   ";

const COLOR: &str = "\x1b[36m";
const BOLD_COLOR: &str = "\x1b[1;36m";
const RESET: &str = "\x1b[0m";

/// What the banner reports: the session vz is about to start.
pub struct Session<'a> {
    pub user: &'a str,
    pub home: &'a Path,
    pub repo_root: &'a Path,
    pub branch: Option<&'a str>,
    /// The configuration files read, global first.
    pub config_files: &'a [&'a Path],
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
}

/// The title, `user@repository`.
pub fn title(session: &Session) -> String {
    let repo = session
        .repo_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{}@{repo}", session.user)
}

/// One `(key, value)` per line under the title.
pub fn facts(session: &Session) -> Vec<(&'static str, String)> {
    let home = session.home;
    let or_none = |text: String| match text.is_empty() {
        true => "none".to_owned(),
        false => text,
    };
    let file_names: Vec<String> = session
        .config_files
        .iter()
        .filter_map(|file| file.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    let mounts: Vec<String> = session
        .mounts
        .iter()
        .map(|mount| {
            let mode = if mount.read_only { "ro" } else { "rw" };
            format!("{} ({mode})", tilde(&mount.path, home))
        })
        .collect();
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
    let mut facts = vec![("Repo", tilde(session.repo_root, home))];
    if let Some(branch) = session.branch {
        facts.push(("Branch", branch.to_owned()));
    }
    facts.extend([
        ("Config", or_none(file_names.join(", "))),
        ("Profile", session.profile.unwrap_or("none").to_owned()),
        ("Image", session.image.to_owned()),
        ("Shell", session.shell.unwrap_or("bash, else sh").to_owned()),
        (
            "Sudo",
            match session.sudo {
                true => "yes".to_owned(),
                false => "no, the secure floor".to_owned(),
            },
        ),
        (
            "Docker",
            session
                .docker
                .map_or("no".to_owned(), |socket| socket.display().to_string()),
        ),
        (
            "Network",
            match session.host_network {
                true => "the host's".to_owned(),
                false => "docker's own".to_owned(),
            },
        ),
        ("Mounts", or_none(mounts.join(", "))),
        ("State", state),
        ("Env", env),
    ]);
    facts
}

/// The art, and beside it the title, a rule, and the facts; colored for a terminal.
pub fn render(title: &str, facts: &[(&str, String)], color: bool) -> String {
    let paint = |code: &str, text: &str| match color {
        true => format!("{code}{text}{RESET}"),
        false => text.to_owned(),
    };
    let art: Vec<&str> = ART.lines().collect();
    let art_width = art
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let right: Vec<String> = [paint(BOLD_COLOR, title), "-".repeat(title.chars().count())]
        .into_iter()
        .chain(
            facts
                .iter()
                .map(|(key, value)| format!("{}: {value}", paint(BOLD_COLOR, key))),
        )
        .collect();
    let mut banner = String::new();
    for row in 0..art.len().max(right.len()) {
        let left = format!("{:art_width$}", art.get(row).unwrap_or(&""));
        let line = match right.get(row) {
            Some(fact) => format!("{}{GAP}{fact}", paint(COLOR, &left)),
            None => paint(COLOR, left.trim_end()),
        };
        banner.push_str(line.trim_end());
        banner.push('\n');
    }
    banner.push('\n');
    banner
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn session<'a>(mounts: &'a [HostMount], files: &'a [&'a Path]) -> Session<'a> {
        Session {
            user: "sally",
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
        }
    }

    #[test]
    fn facts__session__one_line_each_in_order() {
        let mounts = [
            HostMount {
                path: PathBuf::from("/home/sally/repos"),
                read_only: true,
            },
            HostMount {
                path: PathBuf::from("/etc/hosts"),
                read_only: false,
            },
        ];
        let files = [
            Path::new("/home/sally/.config/viz-shell/global.yml"),
            Path::new("/home/sally/repos/app/viz-shell.yml"),
        ];

        let facts = facts(&session(&mounts, &files));

        let expected = [
            ("Repo", "~/repos/app"),
            ("Branch", "main"),
            ("Config", "global.yml, viz-shell.yml"),
            ("Profile", "trusted"),
            ("Image", "vz-app:0123456789abcdef"),
            ("Shell", "fish"),
            ("Sudo", "yes"),
            ("Docker", "/run/user/1000/docker.sock"),
            ("Network", "docker's own"),
            ("Mounts", "~/repos (ro), /etc/hosts (rw)"),
            ("State", "2 paths in ~/repos/app/.vz_state"),
            ("Env", "1 variable"),
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
            ("Shell", "bash, else sh"),
            ("Sudo", "no, the secure floor"),
            ("Docker", "no"),
            ("Mounts", "none"),
            ("State", "none"),
            ("Env", "none"),
        ];
        for (key, expected) in expected {
            assert_eq!(value(key), Some(expected), "{key}");
        }
    }

    #[test]
    fn render__facts__beside_the_art_title_first() {
        let facts = [("Repo", "~/repos/app".to_owned())];

        let banner = render("sally@app", &facts, false);

        let lines: Vec<&str> = banner.lines().collect();
        let art: Vec<&str> = ART.lines().collect();
        let column = art.iter().map(|line| line.len()).max().unwrap() + GAP.len();
        assert_eq!(&lines[0][column..], "sally@app");
        assert_eq!(&lines[1][column..], "---------");
        assert_eq!(&lines[2][column..], "Repo: ~/repos/app");
        assert_eq!(lines[3], art[3].trim_end());
        assert!(banner.ends_with("\n\n"), "{banner}");
        assert!(!banner.contains('\x1b'), "{banner}");
    }

    #[test]
    fn render__color__art_and_keys_colored() {
        let facts = [("Repo", "~/repos/app".to_owned())];

        let banner = render("sally@app", &facts, true);

        assert!(banner.contains(&format!("{BOLD_COLOR}Repo{RESET}: ~/repos/app")));
        assert!(banner.starts_with(COLOR), "{banner}");
    }
}
