//! Parsing tests for the subcommand interface.

use crate::cli::{BuildArgs, CheckArgs, Cli, CliCommand};

mod build;
mod commands;

fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
    Cli::parse_from(std::iter::once("sealc").chain(arguments.iter().copied()))
}

fn parsed(arguments: &[&str]) -> Cli {
    parse(arguments).unwrap_or_else(|error| panic!("arguments did not parse: {error}"))
}

fn build_args(arguments: &[&str]) -> BuildArgs {
    match parsed(arguments).command {
        CliCommand::Build(args) => args,
        command => panic!("expected build, got {command:?}"),
    }
}

fn check_args(arguments: &[&str]) -> CheckArgs {
    match parsed(arguments).command {
        CliCommand::Check(args) => args,
        command => panic!("expected check, got {command:?}"),
    }
}
