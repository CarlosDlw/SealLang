//! Human-readable terminal rendering.

use std::fmt::Write as _;
use std::io::{self, Write};

use crate::diagnostic::{Applicability, Diagnostic, Label, LabelStyle};
use crate::emitter::Emitter;
use crate::level::Level;
use seal_span::{SourceMap, Span};
use seal_unicode::{display_width, expand_tabs};

/// When to colorize output.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ColorChoice {
    /// Color when the output is a terminal and the environment allows it.
    #[default]
    Auto,
    /// Always color.
    Always,
    /// Never color.
    Never,
}

/// Resolves a [`ColorChoice`] against terminal detection and the environment.
///
/// `env` is injected so the decision is testable and does not read global state
/// implicitly.
pub fn should_use_color(
    choice: ColorChoice,
    is_terminal: bool,
    env: impl Fn(&str) -> Option<String>,
) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            if !is_terminal {
                return false;
            }
            if env("NO_COLOR").is_some() {
                return false;
            }
            if env("TERM").as_deref() == Some("dumb") {
                return false;
            }
            true
        }
    }
}

const RESET: &str = "\x1b[0m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_YELLOW: &str = "\x1b[1;33m";
const BOLD_CYAN: &str = "\x1b[1;36m";
const BOLD: &str = "\x1b[1m";

/// Renders diagnostics with source snippets.
pub struct RichEmitter<W: Write> {
    writer: W,
    color: bool,
}

impl<W: Write> RichEmitter<W> {
    /// Creates a plain (uncolored) emitter.
    pub fn plain(writer: W) -> Self {
        Self {
            writer,
            color: false,
        }
    }

    /// Creates an emitter with explicit color configuration.
    pub fn with_color(writer: W, color: bool) -> Self {
        Self { writer, color }
    }

    fn paint(&self, style: &str, text: &str) -> String {
        if self.color {
            format!("{style}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }

    fn header(&self, diagnostic: &Diagnostic) -> String {
        let level = diagnostic.level();
        let style = match level {
            Level::Bug | Level::Fatal | Level::Error => BOLD_RED,
            Level::Warning => BOLD_YELLOW,
            Level::Note | Level::Help => BOLD_CYAN,
        };
        let label = self.paint(style, level.as_str());
        let code = diagnostic
            .code()
            .map_or_else(String::new, |code| format!("[{code}]"));
        format!(
            "{label}{code}: {}\n",
            self.paint(BOLD, diagnostic.message())
        )
    }

    /// Renders a single label block. Returns `false` when the label has no
    /// resolvable source location.
    fn render_label(&self, out: &mut String, label: &Label, map: &SourceMap) -> bool {
        let Some(snippet) = map.snippet(label.span) else {
            return false;
        };
        let file = snippet.file();
        let range = snippet.range();
        let lines = file.lines();
        let start_line = lines.line_of(range.start);
        let end_line = if range.end > range.start {
            lines.line_of(range.end - 1)
        } else {
            start_line
        };
        let last_line_number = end_line + 1;
        let width = last_line_number.to_string().len();
        let indent = " ".repeat(width);

        if let Some(location) = map.span_to_string(label.span) {
            let _ = writeln!(out, "{indent}--> {location}");
            let _ = writeln!(out, "{indent} |");
        }

        let marker_style = if label.style == LabelStyle::Primary {
            '^'
        } else {
            '-'
        };
        let marker_color = match label.style {
            LabelStyle::Primary => BOLD_RED,
            LabelStyle::Secondary => BOLD_CYAN,
        };

        let mut line_number = start_line;
        while line_number <= end_line {
            let Some(text) = file.line_text(line_number) else {
                break;
            };
            let display = expand_tabs(text);
            let _ = writeln!(
                out,
                "{:>width$} | {}",
                line_number + 1,
                display,
                width = width
            );

            let line_start = lines.line_start(line_number).unwrap_or(0);
            let line_end = line_start + text.len() as u32;

            let marker_start_byte = range.start.max(line_start).min(line_end);
            let marker_end_byte = if line_number == end_line {
                range.end.max(line_start).min(line_end)
            } else {
                line_end
            };

            let before = &text[..(marker_start_byte - line_start) as usize];
            let span_text = &text[(marker_start_byte - line_start) as usize
                ..(marker_end_byte - line_start) as usize];
            let marker_columns = display_width(&expand_tabs(span_text)).max(1);
            let marker = marker_style.to_string().repeat(marker_columns);
            let marker = self.paint(marker_color, &marker);
            let message = if line_number == end_line {
                label.message.as_str()
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "{indent} | {:width$}{marker} {message}",
                "",
                width = display_width(&expand_tabs(before)),
            );
            if line_number < end_line && end_line > start_line + 1 {
                let _ = writeln!(out, "{indent} | ...");
                line_number = end_line - 1;
            }
            line_number += 1;
        }
        true
    }

    fn render_suggestion(&self, out: &mut String, diagnostic: &Diagnostic, map: &SourceMap) {
        for suggestion in diagnostic.suggestions() {
            let kind = match suggestion.applicability {
                Applicability::MachineApplicable => "help",
                Applicability::MaybeIncorrect => "help",
                Applicability::HasPlaceholders => "help",
            };
            let _ = writeln!(out, "   = {kind}: {}", suggestion.message);
            for (span, replacement) in &suggestion.replacements {
                self.render_replacement(out, *span, replacement, map);
            }
        }
    }

    fn render_replacement(&self, out: &mut String, span: Span, replacement: &str, map: &SourceMap) {
        let Some(snippet) = map.snippet(span) else {
            return;
        };
        let file = snippet.file();
        let range = snippet.range();
        let start_line = file.lines().line_of(range.start);
        let end_line = if range.end > range.start {
            file.lines().line_of(range.end - 1)
        } else {
            start_line
        };
        let line_numbers = end_line + 1;
        let width = line_numbers.to_string().len();
        for line in start_line..=end_line {
            if let Some(text) = file.line_text(line) {
                let _ = writeln!(
                    out,
                    "{:>width$} | {} {}",
                    line + 1,
                    self.paint("\x1b[31m", "-"),
                    expand_tabs(text),
                    width = width
                );
            }
        }
        for line in replacement.lines() {
            let _ = writeln!(
                out,
                "{:>width$} | {} {}",
                start_line + 1,
                self.paint("\x1b[32m", "+"),
                expand_tabs(line),
                width = width
            );
        }
    }

    fn render(&self, out: &mut String, diagnostic: &Diagnostic, map: &SourceMap) {
        out.push_str(&self.header(diagnostic));

        let mut rendered_label = false;
        for label in diagnostic.labels() {
            rendered_label |= self.render_label(out, label, map);
        }
        if !rendered_label && !diagnostic.labels().is_empty() {
            let _ = writeln!(out, "   = note: diagnostic has no source location");
        }

        for note in diagnostic.notes() {
            let _ = writeln!(out, "   = note: {note}");
        }
        for help in diagnostic.helps() {
            let _ = writeln!(out, "   = help: {help}");
        }
        self.render_suggestion(out, diagnostic, map);

        if diagnostic.level() == Level::Bug {
            let _ = writeln!(
                out,
                "   = note: this is a bug in the Seal compiler, not in your program; please report it"
            );
        }
        let _ = writeln!(out);
    }
}

impl<W: Write + Send> Emitter for RichEmitter<W> {
    fn emit(&mut self, diagnostic: &Diagnostic, source_map: &SourceMap) -> io::Result<()> {
        let mut rendered = String::new();
        self.render(&mut rendered, diagnostic, source_map);
        self.writer.write_all(rendered.as_bytes())?;
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{ColorChoice, RichEmitter, should_use_color};
    use crate::Diagnostic;
    use crate::diagnostic::{Applicability, LabelStyle, Suggestion};
    use crate::emitter::Emitter;
    use crate::level::Level;
    use seal_span::{FileName, SourceMap};

    fn fixture() -> (SourceMap, seal_span::Span) {
        let map = SourceMap::new();
        let id = map
            .add_file(
                FileName::real("examples/demo.sl"),
                "func main() -> i32 {\n  let x: i32 = ;\n  x\n}\n".to_owned(),
            )
            .expect("file registers");
        let span = map.span(id, 36..37).expect("valid span");
        (map, span)
    }

    fn render(diagnostic: &Diagnostic) -> String {
        let (map, span) = fixture();
        let diagnostic = diagnostic.clone();
        let _ = span;
        let mut buffer = Vec::new();
        let mut emitter = RichEmitter::plain(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        String::from_utf8(buffer).expect("output is UTF-8")
    }

    #[test]
    fn renders_error_with_location_and_label() {
        let (map, span) = fixture();
        let diagnostic = Diagnostic::new(Level::Error, "expected expression")
            .with_code(crate::DiagCode::new("E1001"))
            .with_label(span, "expected expression here", LabelStyle::Primary);
        let mut buffer = Vec::new();
        let mut emitter = RichEmitter::plain(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        assert!(output.starts_with("error[E1001]: expected expression\n"));
        assert!(output.contains("--> examples/demo.sl:2:16"));
        assert!(output.contains("let x: i32 = ;"));
        assert!(output.contains("^ expected expression here"));
    }

    #[test]
    fn renders_notes_and_helps() {
        let diagnostic = Diagnostic::new(Level::Warning, "unused variable")
            .with_note("the variable is never read")
            .with_help("prefix the name with `_`");
        let output = render(&diagnostic);
        assert!(output.contains("= note: the variable is never read"));
        assert!(output.contains("= help: prefix the name with `_`"));
    }

    #[test]
    fn renders_suggestions_as_diff() {
        let (map, span) = fixture();
        let diagnostic =
            Diagnostic::new(Level::Error, "missing value").with_suggestion(Suggestion {
                message: "provide a value".to_owned(),
                applicability: Applicability::MachineApplicable,
                replacements: vec![(span, "0".to_owned())],
            });
        let mut buffer = Vec::new();
        let mut emitter = RichEmitter::plain(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        assert!(output.contains("= help: provide a value"));
        assert!(output.contains("-   let x: i32 = ;"));
        assert!(output.contains("+ 0"));
    }

    #[test]
    fn tabs_and_wide_characters_align_markers() {
        let map = SourceMap::new();
        let id = map
            .add_file(FileName::virtual_name("t"), "日\tx = ;\n".to_owned())
            .expect("file registers");
        let span = map.span(id, 5..6).expect("valid span");
        let diagnostic =
            Diagnostic::new(Level::Error, "bad").with_label(span, "here", LabelStyle::Primary);
        let mut buffer = Vec::new();
        let mut emitter = RichEmitter::plain(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        // "日" is two columns wide and the tab expands to two spaces, so the
        // marker starts at display column 5 (after 4 columns of prefix).
        let marker_line = output
            .lines()
            .find(|line| line.contains('^'))
            .expect("marker line exists");
        let caret = marker_line.find('^').expect("caret exists");
        // One space of indent, the pipe, one separator space, then the five
        // display columns occupied by "日  x".
        assert_eq!(marker_line, "  |      ^ here");
        assert_eq!(caret, 9);
    }

    #[test]
    fn multiline_spans_are_summarized() {
        let map = SourceMap::new();
        let id = map
            .add_file(
                FileName::virtual_name("m"),
                "let a = 1;\nlet b = 2;\nlet c = 3;\n".to_owned(),
            )
            .expect("file registers");
        let span = map.span(id, 4..28).expect("valid span");
        let diagnostic = Diagnostic::new(Level::Error, "bad block").with_label(
            span,
            "spans lines",
            LabelStyle::Primary,
        );
        let mut buffer = Vec::new();
        let mut emitter = RichEmitter::plain(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        assert!(output.contains("1 | let a = 1;"));
        assert!(output.contains("..."));
        assert!(output.contains("3 | let c = 3;"));
        assert!(output.contains("spans lines"));
        assert!(
            !output.contains("2 | let b = 2;"),
            "middle lines are elided"
        );
    }

    #[test]
    fn secondary_labels_use_dashes() {
        let (map, span) = fixture();
        let diagnostic = Diagnostic::new(Level::Warning, "careful").with_label(
            span,
            "second opinion",
            LabelStyle::Secondary,
        );
        let mut buffer = Vec::new();
        let mut emitter = RichEmitter::plain(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        assert!(output.contains("- second opinion"));
    }

    #[test]
    fn empty_spans_still_get_a_marker() {
        let (map, _) = fixture();
        let file_id = map.files().first().expect("fixture has a file").0;
        let empty = map.span(file_id, 0..0).expect("valid empty span");
        let diagnostic = Diagnostic::new(Level::Error, "here").with_label(
            empty,
            "insert here",
            LabelStyle::Primary,
        );
        let mut buffer = Vec::new();
        let mut emitter = RichEmitter::plain(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        assert!(output.contains("^ insert here"));
    }

    #[test]
    fn every_level_has_a_header() {
        for (level, prefix) in [
            (Level::Error, "error"),
            (Level::Warning, "warning"),
            (Level::Note, "note"),
            (Level::Help, "help"),
            (Level::Fatal, "fatal error"),
            (Level::Bug, "internal compiler error"),
        ] {
            let output = render(&Diagnostic::new(level, "message"));
            assert!(
                output.starts_with(&format!("{prefix}: message")),
                "{level:?}"
            );
        }
    }

    #[test]
    fn color_resolution_respects_environment() {
        assert!(should_use_color(ColorChoice::Always, false, |_| None));
        assert!(!should_use_color(ColorChoice::Never, true, |_| None));
        assert!(should_use_color(ColorChoice::Auto, true, |_| None));
        assert!(!should_use_color(ColorChoice::Auto, false, |_| None));
        assert!(!should_use_color(ColorChoice::Auto, true, |key| {
            (key == "NO_COLOR").then(|| "1".to_owned())
        }));
        assert!(!should_use_color(ColorChoice::Auto, true, |key| {
            (key == "TERM").then(|| "dumb".to_owned())
        }));
    }

    #[test]
    fn writes_to_failing_writer_propagate() {
        struct Failing;
        impl io::Write for Failing {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let map = SourceMap::new();
        let diagnostic = Diagnostic::new(Level::Error, "boom");
        let mut emitter = RichEmitter::plain(Failing);
        assert!(emitter.emit(&diagnostic, &map).is_err());
    }
}
