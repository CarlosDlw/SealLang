//! `seal stdlib`: inspect and verify the installed standard library.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use seal_stdlib::Stdlib;

use crate::ui;

/// `seal stdlib`.
#[derive(Debug, Args)]
pub(crate) struct StdlibArgs {
    /// The standard-library operation.
    #[command(subcommand)]
    pub(crate) command: StdlibCommand,
}

/// Standard-library operations.
#[derive(Debug, Subcommand)]
pub(crate) enum StdlibCommand {
    /// Print the selected standard-library root.
    Path,
    /// Print the language, version, and digest.
    Version,
    /// Recompute the digest and verify every manifest and source.
    Verify,
}

pub(crate) fn run(args: &StdlibArgs) -> ExitCode {
    let root = match Stdlib::locate() {
        Ok(root) => root,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    if let StdlibCommand::Path = &args.command {
        println!("{}", root.display());
        return ExitCode::SUCCESS;
    }
    let stdlib = match Stdlib::load_unverified(&root) {
        Ok(stdlib) => stdlib,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    match args.command {
        StdlibCommand::Path => {
            println!("{}", stdlib.root().display());
            ExitCode::SUCCESS
        }
        StdlibCommand::Version => {
            println!("language {}", stdlib.language());
            println!("version {}", stdlib.version());
            println!("digest {}", stdlib.digest());
            println!("root {}", stdlib.root().display());
            println!(
                "packages {}",
                stdlib
                    .packages()
                    .iter()
                    .map(|package| package.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for package in stdlib.packages() {
                if let Some(prelude) = &package.prelude {
                    println!("prelude {} {prelude}", package.name);
                }
            }
            ExitCode::SUCCESS
        }
        StdlibCommand::Verify => match stdlib.verify() {
            Ok(()) => {
                println!(
                    "ok: standard library {} {} at `{}` (digest {})",
                    stdlib.language(),
                    stdlib.version(),
                    stdlib.root().display(),
                    stdlib.digest()
                );
                ExitCode::SUCCESS
            }
            Err(error) => ui::error(error.code(), error.message()),
        },
    }
}
