use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::constants::CONFIG_ENV;

/// One shell for every repo.
#[derive(Debug, Parser)]
#[command(name = "viz-shell", version, arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    pub action: Option<Action>,

    /// The configuration to run, after the ones it extends: the
    /// repository's, else the library's. Without it: default
    #[arg(short = 'c', long, value_name = "NAME", env = CONFIG_ENV, global = true)]
    pub config: Option<String>,

    /// Also read this YAML file, as one of the repository's; repeatable.
    /// Paths in it are relative to its folder
    #[arg(short = 'f', long = "file", value_name = "FILE", global = true)]
    pub files: Vec<PathBuf>,

    /// Print the configuration chain, the image chain and the configuration
    /// vz would run with, as YAML, and exit
    #[arg(long, global = true)]
    pub show_effective_config: bool,

    /// Set a variable inside, over every other source: KEY=VALUE, or KEY to
    /// copy the host's; repeatable
    #[arg(short = 'e', long = "env", value_name = "KEY[=VALUE]", global = true)]
    pub env: Vec<String>,

    /// Print the environment's variable names and where each comes from, never
    /// a value, and exit
    #[arg(long, global = true)]
    pub show_env: bool,

    /// A command to run instead of the shell: `vz -- cargo test`
    #[arg(last = true)]
    pub command: Vec<String>,
}

#[derive(Debug, PartialEq, Subcommand)]
pub enum Action {
    /// A shell in a new container; named: `vz new api`, then `vz attach api`
    New {
        /// Names the container; without one, `attach: true` joins a
        /// container of this configuration instead
        name: Option<String>,
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
    /// List the configurations of the repository and the library
    Configs,
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
    /// Inside the container: a hook; become the host user and run it
    #[command(hide = true)]
    AsUser {
        #[arg(last = true, required = true)]
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

    /// Not isolated: `-c` falls back to `VZ_CONFIG`, so in a shell that
    /// exports it this goes red (and bare `vz` says "nothing to run" instead
    /// of help). Unset VZ_CONFIG to run it; see the audit report.
    #[test]
    fn parse__nothing__help_instead() {
        let error = Cli::try_parse_from(["vz"]).unwrap_err();

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn parse__file_with_configs__before_or_after_it() {
        for args in [
            ["vz", "-f", "other.vz.yml", "configs"],
            ["vz", "configs", "-f", "other.vz.yml"],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();

            assert_eq!(cli.files, [PathBuf::from("other.vz.yml")], "{args:?}");
        }
    }

    #[test]
    fn parse__file_repeated__each_in_order_short_or_long() {
        let cli = Cli::try_parse_from(["vz", "-f", "app.vz.yml", "--file", "ci.yml"]).unwrap();

        assert_eq!(
            cli.files,
            [PathBuf::from("app.vz.yml"), PathBuf::from("ci.yml")]
        );
    }

    #[test]
    fn parse__config_short_or_long__names_it() {
        for args in [
            ["vz", "-c", "ci", "--", "true"],
            ["vz", "--config", "ci", "--", "true"],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();

            assert_eq!(cli.config.as_deref(), Some("ci"), "{args:?}");
        }
    }

    #[test]
    fn parse__show_flags_after_a_subcommand__apply() {
        let cli =
            Cli::try_parse_from(["vz", "new", "-c", "ci", "--show-effective-config"]).unwrap();

        assert!(cli.show_effective_config);
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
        let cases: [(&[&str], Action); 10] = [
            (
                &["vz", "new"],
                Action::New {
                    name: None,
                    command: vec![],
                },
            ),
            (
                &["vz", "new", "--", "id"],
                Action::New {
                    name: None,
                    command: strings(&["id"]),
                },
            ),
            (
                &["vz", "new", "api"],
                Action::New {
                    name: Some("api".to_owned()),
                    command: vec![],
                },
            ),
            (
                &["vz", "new", "api", "--", "cargo", "test"],
                Action::New {
                    name: Some("api".to_owned()),
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
            (&["vz", "configs"], Action::Configs),
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
        let error = Cli::try_parse_from(["vz", "kill"]).unwrap_err();

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn parse__config_after_the_subcommand__applies() {
        let cli = Cli::try_parse_from(["vz", "attach", "0", "-c", "trusted"]).unwrap();

        assert_eq!(cli.config.as_deref(), Some("trusted"));
    }
}
