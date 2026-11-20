//! `seal`, the project command-line interface.

mod cli;
#[cfg(test)]
mod tests;
mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run(std::env::args_os())
}
