//! The `sealc` compiler driver library.
//!
//! The binary is a thin wrapper over [`run_from`], which lets the project CLI
//! invoke the compiler in-process without spawning a second executable.

#![warn(missing_docs)]

mod actions;
mod api;
mod cli;
mod doc_text;
mod frontend;
mod options;
mod output;
mod pipeline;
mod report;
mod temp;

use std::ffi::OsString;
use std::panic::AssertUnwindSafe;
use std::process::ExitCode;

use seal_diagnostics::IcePanic;

/// Runs `sealc` with the given argv, including the program name.
pub fn run_from<I, T>(argv: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    install_panic_hook();
    let cli = match cli::Cli::parse_from(argv) {
        Ok(cli) => cli,
        Err(error) => {
            let informational = matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            let _ = error.print();
            return if informational {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
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

fn run(cli: &cli::Cli) -> Result<ExitCode, String> {
    match &cli.command {
        cli::CliCommand::Build(args) => pipeline::build(args),
        cli::CliCommand::Run(args) => pipeline::run(args),
        cli::CliCommand::Check(args) => pipeline::check(args),
        cli::CliCommand::Codegen(args) => pipeline::codegen(args),
        cli::CliCommand::Explain(args) => actions::explain(&args.code),
        cli::CliCommand::EmitRuntime(args) => actions::emit_runtime(&args.path),
        cli::CliCommand::Version => actions::version(),
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.payload().is::<IcePanic>() {
            return;
        }
        previous(info);
    }));
}
