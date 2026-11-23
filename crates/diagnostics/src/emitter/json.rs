//! Newline-delimited JSON output.
//!
//! Each diagnostic is one JSON object on its own line, with a fixed field order
//! and a `schema_version` field. Consumers should ignore unknown fields and
//! reject unknown schema versions.

use std::fmt::Write as _;
use std::io::{self, Write};

use crate::diagnostic::{Applicability, Diagnostic};
use crate::emitter::Emitter;
use crate::level::Level;
use seal_span::SourceMap;

/// Schema version of the JSON diagnostic format.
pub const SCHEMA_VERSION: u32 = 1;

/// Emits one JSON object per diagnostic.
pub struct JsonEmitter<W: Write> {
    writer: W,
}

impl<W: Write> JsonEmitter<W> {
    /// Creates a JSON emitter around `writer`.
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write + Default> Default for JsonEmitter<W> {
    fn default() -> Self {
        Self::new(W::default())
    }
}

impl<W: Write + Send> Emitter for JsonEmitter<W> {
    fn emit(&mut self, diagnostic: &Diagnostic, source_map: &SourceMap) -> io::Result<()> {
        let mut line = String::new();
        line.push('{');
        let _ = write!(line, "\"schema_version\":{SCHEMA_VERSION},");
        let _ = write!(line, "\"level\":\"{}\",", level_name(diagnostic.level()));
        line.push_str("\"code\":");
        match diagnostic.code() {
            Some(code) => write_json_string(&mut line, code.as_str()),
            None => line.push_str("null"),
        }
        line.push(',');
        line.push_str("\"lint\":");
        match diagnostic.lint() {
            Some(lint) => write_json_string(&mut line, lint),
            None => line.push_str("null"),
        }
        line.push(',');
        line.push_str("\"message\":");
        write_json_string(&mut line, diagnostic.message());
        line.push_str(",\"labels\":[");
        for (index, label) in diagnostic.labels().iter().enumerate() {
            if index > 0 {
                line.push(',');
            }
            line.push('{');
            match source_map.snippet(label.span) {
                Some(snippet) => {
                    let file = snippet.file();
                    let range = snippet.range();
                    line.push_str("\"file\":");
                    write_json_string(
                        &mut line,
                        &source_map
                            .file_of(label.span)
                            .and_then(|id| source_map.display_name(id))
                            .unwrap_or_else(|| file.name().to_string()),
                    );
                    let _ = write!(
                        line,
                        ",\"byte_start\":{},\"byte_end\":{}",
                        range.start, range.end
                    );
                    match source_map.line_col(label.span) {
                        Some((_, start, end)) => {
                            let _ = write!(
                                line,
                                ",\"line_start\":{},\"column_start\":{},\"line_end\":{},\"column_end\":{}",
                                start.line, start.column, end.line, end.column
                            );
                        }
                        None => {
                            line.push_str(
                                ",\"line_start\":null,\"column_start\":null,\"line_end\":null,\"column_end\":null",
                            );
                        }
                    }
                }
                None => {
                    line.push_str(
                        "\"file\":null,\"byte_start\":null,\"byte_end\":null,\
                         \"line_start\":null,\"column_start\":null,\
                         \"line_end\":null,\"column_end\":null",
                    );
                }
            }
            let _ = write!(
                line,
                ",\"is_primary\":{},\"label\":",
                label.style == crate::diagnostic::LabelStyle::Primary
            );
            write_json_string(&mut line, &label.message);
            line.push('}');
        }
        line.push(']');
        line.push_str(",\"notes\":[");
        for (index, note) in diagnostic.notes().iter().enumerate() {
            if index > 0 {
                line.push(',');
            }
            write_json_string(&mut line, note);
        }
        line.push(']');
        line.push_str(",\"helps\":[");
        for (index, help) in diagnostic.helps().iter().enumerate() {
            if index > 0 {
                line.push(',');
            }
            write_json_string(&mut line, help);
        }
        line.push(']');
        line.push_str(",\"suggestions\":[");
        for (index, suggestion) in diagnostic.suggestions().iter().enumerate() {
            if index > 0 {
                line.push(',');
            }
            line.push('{');
            line.push_str("\"message\":");
            write_json_string(&mut line, &suggestion.message);
            let _ = write!(
                line,
                ",\"applicability\":\"{}\",\"replacements\":[",
                applicability_name(suggestion.applicability)
            );
            for (replacement_index, (span, text)) in suggestion.replacements.iter().enumerate() {
                if replacement_index > 0 {
                    line.push(',');
                }
                line.push('{');
                match source_map.snippet(*span) {
                    Some(snippet) => {
                        let range = snippet.range();
                        line.push_str("\"file\":");
                        write_json_string(
                            &mut line,
                            &source_map
                                .file_of(*span)
                                .and_then(|id| source_map.display_name(id))
                                .unwrap_or_default(),
                        );
                        let _ = write!(
                            line,
                            ",\"byte_start\":{},\"byte_end\":{},\"text\":",
                            range.start, range.end
                        );
                    }
                    None => {
                        line.push_str(
                            "\"file\":null,\"byte_start\":null,\"byte_end\":null,\"text\":",
                        );
                    }
                }
                write_json_string(&mut line, text);
                line.push('}');
            }
            line.push_str("]}");
        }
        line.push_str("]}\n");
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Bug => "bug",
        Level::Fatal => "fatal",
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Note => "note",
        Level::Help => "help",
    }
}

fn applicability_name(applicability: Applicability) -> &'static str {
    match applicability {
        Applicability::MachineApplicable => "machine-applicable",
        Applicability::MaybeIncorrect => "maybe-incorrect",
        Applicability::HasPlaceholders => "has-placeholders",
    }
}

/// Writes a JSON string with the mandatory escapes. Valid UTF-8 is passed
/// through unchanged; control characters are escaped.
fn write_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::{JsonEmitter, SCHEMA_VERSION, write_json_string};
    use crate::Diagnostic;
    use crate::diagnostic::{Applicability, LabelStyle, Suggestion};
    use crate::emitter::Emitter;
    use crate::level::Level;
    use seal_span::{FileName, SourceMap};

    #[test]
    fn emits_one_object_per_diagnostic_with_stable_fields() {
        let map = SourceMap::new();
        let id = map
            .add_file(FileName::virtual_name("t"), "let x = ;\n".to_owned())
            .expect("file registers");
        let span = map.span(id, 8..9).expect("valid span");
        let diagnostic = Diagnostic::new(Level::Error, "expected expression")
            .with_code(crate::DiagCode::new("E1001"))
            .with_label(span, "here", LabelStyle::Primary)
            .with_note("a note")
            .with_suggestion(Suggestion {
                message: "insert a value".to_owned(),
                applicability: Applicability::MachineApplicable,
                replacements: vec![(span, "0".to_owned())],
            });
        let mut buffer = Vec::new();
        let mut emitter = JsonEmitter::new(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        assert_eq!(output.lines().count(), 1);
        assert!(output.starts_with(&format!(
            "{{\"schema_version\":{SCHEMA_VERSION},\"level\":\"error\",\"code\":\"E1001\",\"lint\":null"
        )));
        assert!(output.contains("\"file\":\"<t>\""));
        assert!(output.contains("\"line_start\":1"));
        assert!(output.contains("\"is_primary\":true"));
        assert!(output.contains("\"label\":\"here\""));
        assert!(output.contains("\"notes\":[\"a note\"]"));
        assert!(output.contains("\"applicability\":\"machine-applicable\""));
        assert!(output.contains("\"text\":\"0\""));
        assert!(output.ends_with("]}\n"));
    }

    #[test]
    fn strings_are_escaped() {
        let mut output = String::new();
        write_json_string(&mut output, "a\"b\\c\nd\te\u{1}");
        assert_eq!(output, "\"a\\\"b\\\\c\\nd\\te\\u0001\"");
    }

    #[test]
    fn labels_without_source_locations_use_nulls() {
        let map = SourceMap::new();
        let span = seal_span::Span::default();
        let diagnostic = Diagnostic::new(Level::Warning, "no span").with_label(
            span,
            "unknown",
            LabelStyle::Secondary,
        );
        let mut buffer = Vec::new();
        let mut emitter = JsonEmitter::new(&mut buffer);
        emitter.emit(&diagnostic, &map).expect("emit succeeds");
        let output = String::from_utf8(buffer).expect("output is UTF-8");
        assert!(output.contains("\"file\":null"));
        assert!(output.contains("\"is_primary\":false"));
    }
}
