//! Terminal color policy for status output.
//!
//! Color is opt-in by terminal detection, and `NO_COLOR` or `--color=never`
//! always win. `--color=always` is honored even when stdout is redirected so
//! tests and log collectors can request ANSI output explicitly.

use std::io::IsTerminal;

use crate::cli::ColorChoice;

const RESET: &str = "\x1b[0m";

/// Returns whether stdout should use ANSI color.
fn stdout_enabled(choice: ColorChoice) -> bool {
    enabled(choice, std::io::stdout().is_terminal())
}

/// Returns whether ANSI color is enabled for the chosen terminal state.
fn enabled(choice: ColorChoice, terminal: bool) -> bool {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let clicolor_disabled = std::env::var_os("CLICOLOR").is_some_and(|value| value == "0");
    supported(choice, terminal, no_color || clicolor_disabled)
}

/// The pure color decision, separated so it can be tested without a terminal.
fn supported(choice: ColorChoice, terminal: bool, no_color: bool) -> bool {
    if no_color {
        return false;
    }
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => terminal,
    }
}

/// Styles `text` in bold when color is enabled.
pub(crate) fn bold(choice: ColorChoice, text: &str) -> String {
    paint(choice, "1", text)
}

/// Styles `text` in dim gray when color is enabled.
pub(crate) fn dim(choice: ColorChoice, text: &str) -> String {
    paint(choice, "2", text)
}

fn paint(choice: ColorChoice, code: &str, text: &str) -> String {
    if stdout_enabled(choice) {
        format!("\x1b[{code}m{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::ColorChoice;

    use super::supported;

    #[test]
    fn auto_colors_only_a_terminal() {
        assert!(supported(ColorChoice::Auto, true, false));
        assert!(!supported(ColorChoice::Auto, false, false));
    }

    #[test]
    fn explicit_choices_override_terminal_detection() {
        assert!(supported(ColorChoice::Always, false, false));
        assert!(!supported(ColorChoice::Never, true, false));
    }

    #[test]
    fn no_color_disables_every_choice() {
        assert!(!supported(ColorChoice::Always, true, true));
        assert!(!supported(ColorChoice::Auto, true, true));
    }
}
