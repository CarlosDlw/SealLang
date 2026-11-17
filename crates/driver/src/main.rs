//! `sealc`, the Seal compiler driver.
//!
//! The driver owns process-level concerns only: command-line parsing, exit
//! codes, panic handling, and phase wiring. Language analysis lives in the phase
//! crates.

mod actions;
mod cli;
mod options;
mod output;
mod pipeline;
mod report;
mod temp;

use std::panic::AssertUnwindSafe;
use std::process::ExitCode;

use seal_diagnostics::IcePanic;

use crate::cli::CliCommand;

fn main() -> ExitCode {
    install_panic_hook();
    let cli = match cli::Cli::parse_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    match std::panic::catch_unwind(AssertUnwindSafe(|| run(&cli))) {
        Ok(Ok(code)) => code,
        Ok(Err(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
        Err(_) => ExitCode::from(101),
    }
}

/// Runs the action the arguments select.
fn run(cli: &cli::Cli) -> Result<ExitCode, String> {
    match &cli.command {
        CliCommand::Build(args) => pipeline::build(args),
        CliCommand::Run(args) => pipeline::run(args),
        CliCommand::Check(args) => pipeline::check(args),
        CliCommand::Explain(args) => actions::explain(&args.code),
        CliCommand::EmitRuntime(args) => actions::emit_runtime(&args.path),
        CliCommand::Version => actions::version(),
    }
}

/// Suppresses the default panic report for internal compiler errors; the
/// diagnostic emitter already printed a structured report.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.payload().is::<IcePanic>() {
            return;
        }
        previous(info);
    }));
}
