use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::constants::PROFILE_ENV;

/// One shell for every repo.
#[derive(Debug, Parser)]
#[command(name = "viz-shell", version, args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub internal: Option<Internal>,

    /// The configuration to use instead of the one at the git root
    /// (viz-shell.yml, …, vz.yaml). Paths in it are relative to its folder.
    #[arg(short = 'c', long)]
    pub config_file: Option<PathBuf>,

    /// A profile from vz.yml, applied on top of its root
    #[arg(long, env = PROFILE_ENV)]
    pub profile: Option<String>,

    /// Print the configuration vz would run with, as YAML, and exit
    #[arg(long)]
    pub show_effective_config: bool,

    /// Set a variable inside, over every other source: KEY=VALUE, or KEY to
    /// copy the host's; repeatable
    #[arg(short = 'e', long = "env", value_name = "KEY[=VALUE]")]
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
pub enum Internal {
    /// Inside the container: add the host user, then run the command as it
    #[command(hide = true)]
    Entrypoint {
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

        let expected = Internal::Entrypoint {
            command: strings(&["bash", "-l"]),
        };
        assert_eq!(cli.internal, Some(expected));
    }
}
