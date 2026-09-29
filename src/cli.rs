use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::constants::PROFILE_ENV;

/// One shell for every repo.
#[derive(Debug, Parser)]
#[command(name = "viz-shell", version)]
pub struct Cli {
    #[command(subcommand)]
    pub action: Option<Action>,

    /// The repository configuration to use instead of the one at the git
    /// root (viz-shell.yml, …, vz.yaml). Paths in it are relative to its folder.
    #[arg(short = 'c', long, global = true)]
    pub config_file: Option<PathBuf>,

    /// A profile from vz.yml, applied on top of its root
    #[arg(long, env = PROFILE_ENV, global = true)]
    pub profile: Option<String>,

    /// Print the configuration vz would run with, as YAML, and exit
    #[arg(long)]
    pub show_effective_config: bool,

    /// Set a variable inside, over every other source: KEY=VALUE, or KEY to
    /// copy the host's; repeatable
    #[arg(short = 'e', long = "env", value_name = "KEY[=VALUE]", global = true)]
    pub env: Vec<String>,

    /// Print the environment's variable names and where each comes from, never
    /// a value, and exit
    #[arg(long)]
    pub show_env: bool,

    /// A command to run instead of the shell: `vz -- cargo test`
    #[arg(last = true)]
    pub command: Vec<String>,
}

#[derive(Debug, PartialEq, Subcommand)]
pub enum Action {
    /// Start a named container: `vz new api`, then `vz attach api`
    New {
        name: String,
        /// A command to run instead of the shell
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Attach to a container of this repository, by index or name; without
    /// one, to the only one running
    #[command(visible_alias = "at")]
    Attach {
        /// An index, as in vz-0-app, or a name from `vz new`
        target: Option<String>,
        /// A command to run instead of the shell
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// List this repository's containers
    Ls {
        /// Every repository's
        #[arg(long)]
        all: bool,
    },
    /// Remove containers of this repository, by index or name
    Kill {
        /// Indexes, as in vz-0-app, or names from `vz new`
        #[arg(required_unless_present = "all")]
        targets: Vec<String>,
        /// Every container of this repository
        #[arg(long, conflicts_with = "targets")]
        all: bool,
    },
    /// List the profiles of the global and the repository configuration
    Profiles,
    /// Inside the container: add the host user, then run the command as it,
    /// or hold for shells to attach
    #[command(hide = true)]
    Entrypoint {
        #[arg(long, conflicts_with = "command")]
        hold: bool,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Inside the container: once the entrypoint is ready, become the host
    /// user and run the command
    #[command(hide = true)]
    Enter {
        #[arg(last = true)]
        command: Vec<String>,
    },
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn parse__config_file_with_profiles__before_or_after_it() {
        for args in [
            ["vz", "-c", "other.yml", "profiles"],
            ["vz", "profiles", "-c", "other.yml"],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();

            assert_eq!(cli.action, Some(Action::Profiles), "{args:?}");
            assert_eq!(
                cli.config_file,
                Some(PathBuf::from("other.yml")),
                "{args:?}"
            );
        }
    }

    #[test]
    fn parse__config_file_short_and_long__same_path() {
        let short = Cli::try_parse_from(["vz", "-c", "examples/state/vz.yml"]).unwrap();
        let long = Cli::try_parse_from(["vz", "--config-file", "examples/state/vz.yml"]).unwrap();

        let expected = Some(PathBuf::from("examples/state/vz.yml"));
        assert_eq!(
            (short.config_file, long.config_file),
            (expected.clone(), expected)
        );
    }

    #[test]
    fn parse__profile__names_it() {
        let cli = Cli::try_parse_from(["vz", "--profile", "ci", "--", "true"]).unwrap();

        assert_eq!(cli.profile.as_deref(), Some("ci"));
    }

    #[test]
    fn parse__env_repeated__kept_in_order() {
        let cli = Cli::try_parse_from(["vz", "-e", "A=1", "--env", "GH_TOKEN"]).unwrap();

        assert_eq!(cli.env, strings(&["A=1", "GH_TOKEN"]));
    }

    #[test]
    fn parse__command_after_dashes__is_command() {
        let cli = Cli::try_parse_from(["vz", "--", "id", "-u"]).unwrap();

        assert_eq!(cli.command, strings(&["id", "-u"]));
    }

    #[test]
    fn parse__entrypoint__carries_its_command() {
        let cli = Cli::try_parse_from(["vz", "entrypoint", "--", "bash", "-l"]).unwrap();

        let expected = Action::Entrypoint {
            hold: false,
            command: strings(&["bash", "-l"]),
        };
        assert_eq!(cli.action, Some(expected));
    }

    #[test]
    fn parse__sessions__each_command() {
        let cases: [(&[&str], Action); 7] = [
            (
                &["vz", "new", "api"],
                Action::New {
                    name: "api".to_owned(),
                    command: vec![],
                },
            ),
            (
                &["vz", "new", "api", "--", "cargo", "test"],
                Action::New {
                    name: "api".to_owned(),
                    command: strings(&["cargo", "test"]),
                },
            ),
            (
                &["vz", "attach"],
                Action::Attach {
                    target: None,
                    command: vec![],
                },
            ),
            (
                &["vz", "at", "1", "--", "id"],
                Action::Attach {
                    target: Some("1".to_owned()),
                    command: strings(&["id"]),
                },
            ),
            (&["vz", "ls", "--all"], Action::Ls { all: true }),
            (
                &["vz", "kill", "0", "api"],
                Action::Kill {
                    targets: strings(&["0", "api"]),
                    all: false,
                },
            ),
            (
                &["vz", "kill", "--all"],
                Action::Kill {
                    targets: vec![],
                    all: true,
                },
            ),
        ];
        for (args, expected) in cases {
            let cli = Cli::try_parse_from(args).unwrap();

            assert_eq!(cli.action, Some(expected), "{args:?}");
        }
    }

    #[test]
    fn parse__kill_without_targets__refused() {
        assert!(Cli::try_parse_from(["vz", "kill"]).is_err());
    }

    #[test]
    fn parse__profile_after_the_subcommand__applies() {
        let cli = Cli::try_parse_from(["vz", "attach", "0", "--profile", "trusted"]).unwrap();

        assert_eq!(cli.profile.as_deref(), Some("trusted"));
    }
}
