//! Tests for command selection, help, and informational actions.

use clap::CommandFactory;
use clap::error::ErrorKind;

use crate::cli::{Cli, CliCommand};

use super::{parse, parsed};

#[test]
fn unknown_commands_are_rejected() {
    let error = parse(&["frobnicate"]).expect_err("unknown commands are rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn a_command_is_required() {
    let error = parse(&[]).expect_err("a command is required");
    assert!(
        matches!(
            error.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand | ErrorKind::MissingSubcommand
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn help_and_version_stop_parsing_successfully() {
    assert_eq!(
        parse(&["--help"]).expect_err("help stops parsing").kind(),
        ErrorKind::DisplayHelp
    );
    assert_eq!(
        parse(&["--version"])
            .expect_err("version stops parsing")
            .kind(),
        ErrorKind::DisplayVersion
    );
    assert_eq!(
        parse(&["help"]).expect_err("help stops parsing").kind(),
        ErrorKind::DisplayHelp
    );
    assert_eq!(
        parse(&["help", "build"])
            .expect_err("command help stops parsing")
            .kind(),
        ErrorKind::DisplayHelp
    );
}

#[test]
fn informational_commands_do_not_require_an_input() {
    assert!(matches!(parsed(&["version"]).command, CliCommand::Version));
    let explain = parsed(&["explain", "E0001"]);
    assert!(matches!(
        explain.command,
        CliCommand::Explain(crate::cli::ExplainArgs { code }) if code == "E0001"
    ));
    let runtime = parsed(&["emit-runtime", "seal_rt.a"]);
    assert!(matches!(
        runtime.command,
        CliCommand::EmitRuntime(crate::cli::EmitRuntimeArgs { path })
            if path.as_os_str() == "seal_rt.a"
    ));
}

#[test]
fn run_is_reserved_and_parses_its_input() {
    let cli = parsed(&["run", "main.sl"]);
    assert!(matches!(
        cli.command,
        CliCommand::Run(crate::cli::RunArgs { input })
            if input.as_os_str() == "main.sl"
    ));
}

#[test]
fn top_level_help_lists_every_command() {
    let help = Cli::command().render_long_help().to_string();
    for expected in [
        "build",
        "run",
        "check",
        "explain",
        "emit-runtime",
        "version",
        "help",
    ] {
        assert!(help.contains(expected), "help is missing `{expected}`");
    }
}
