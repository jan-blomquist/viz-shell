use clap::{Parser, Subcommand};

/// One shell for every repo.
#[derive(Debug, Parser)]
#[command(name = "vz", version, args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub internal: Option<Internal>,

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
    fn parse__bare__is_shell() {
        let cli = Cli::try_parse_from(["vz"]).unwrap();

        assert_eq!((cli.internal, cli.command), (None, vec![]));
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
