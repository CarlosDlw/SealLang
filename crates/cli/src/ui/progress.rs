//! One-line action status and artifact output.

use crate::cli::{Cli, MessageFormat};

use super::color;

/// Prints a human status line unless output is quiet or machine-readable.
pub(crate) fn status(cli: &Cli, message: &str) {
    if human(cli) {
        println!("{}", color::bold(cli.color, message));
    }
}

/// Prints an indented detail under the preceding status line.
pub(crate) fn detail(cli: &Cli, message: &str) {
    if human(cli) {
        println!("  {}", color::dim(cli.color, message));
    }
}

/// Prints a verbose trace to stderr, independently of quiet stdout.
pub(crate) fn verbose(cli: &Cli, message: &str) {
    if cli.verbose {
        eprintln!("{}", color::dim(cli.color, message));
    }
}

fn human(cli: &Cli) -> bool {
    visible(cli.quiet, cli.message_format)
}

fn visible(quiet: bool, format: MessageFormat) -> bool {
    !quiet && format == MessageFormat::Human
}

#[cfg(test)]
mod tests {
    use crate::cli::MessageFormat;

    use super::visible;

    #[test]
    fn quiet_and_json_suppress_human_status() {
        assert!(visible(false, MessageFormat::Human));
        assert!(!visible(true, MessageFormat::Human));
        assert!(!visible(false, MessageFormat::Json));
    }
}
