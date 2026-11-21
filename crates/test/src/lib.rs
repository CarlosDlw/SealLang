//! The Seal test and benchmark runner.
//!
//! Discovery and execution are separate phases. [`discover`] runs each compiled
//! harness with `--list`; the runner then schedules one process per test with
//! [`run`], applying filters, sharding, retries, and timeouts. A test failure
//! never stops the runner from reporting other results unless `fail_fast` is
//! enabled.
//!
//! Reports are deliberately small and stable: JUnit XML for CI and NDJSON for
//! tools. Neither format parses human output.

#![warn(missing_docs)]

mod discover;
mod error;
mod filter;
mod model;
mod process;
mod report;
mod runner;

pub use discover::discover;
pub use error::TestError;
pub use filter::{Partition, Scheduled, Selection};
pub use model::{Executable, Kind, Outcome, Report, Status, Test};
pub use report::{json, junit, write_json, write_junit};
pub use runner::{Options, Progress, run};
