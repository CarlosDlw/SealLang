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

use std::panic::AssertUnwindSafe;
use std::process::ExitCode;

use seal_diagnostics::IcePanic;

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
    if let Some(code) = &cli.explain {
        return actions::explain(code);
    }
    if let Some(path) = &cli.runtime.emit_runtime {
        return actions::emit_runtime(path);
    }
    pipeline::compile(cli)
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
