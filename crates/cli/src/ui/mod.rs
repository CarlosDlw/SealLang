//! Human-facing output shared by the project commands.
//!
//! Status lines respect `--quiet`, `--message-format`, and the terminal color
//! policy. Diagnostics always go to stderr so machine-readable stdout stays
//! clean.

mod color;
mod error;
mod progress;

pub(crate) use color::{green, red, yellow};
pub(crate) use error::{Failure, error, exit_code, failure, warning};
pub(crate) use progress::{detail, status, verbose};
