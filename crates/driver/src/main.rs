//! `sealc`, the Seal compiler driver.

use std::process::ExitCode;

fn main() -> ExitCode {
    seal_driver::run_from(std::env::args_os())
}
