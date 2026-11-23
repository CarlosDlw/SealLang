//! The structured diagnostic model.
//!
//! Emitters consume this model; they never parse messages. That separation is
//! what makes the JSON output a stable machine interface and lets the human
//! renderer change without touching diagnostics.

use crate::code::DiagCode;
use crate::level::Level;
use seal_span::Span;

/// Whether a label is the main subject of the diagnostic or supporting context.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum LabelStyle {
    /// The primary location; the diagnostic is about this span.
    Primary,
    /// Supporting context.
    Secondary,
}

/// A message attached to a source span.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Label {
    /// The annotated span.
    pub span: Span,
    /// The label text. An empty label renders only the marker.
    pub message: String,
    /// Whether the label is primary or secondary.
    pub style: LabelStyle,
}

/// How safe it is to apply a suggestion automatically.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Applicability {
    /// The suggestion is guaranteed to be valid Rust-level equivalent.
    MachineApplicable,
    /// The suggestion is likely correct but not guaranteed.
    MaybeIncorrect,
    /// The suggestion contains placeholders such as `/* type */`.
    HasPlaceholders,
}

/// A text replacement suggested to the user.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Suggestion {
    /// Explanation shown before the replacement.
    pub message: String,
    /// How safe it is to apply automatically.
    pub applicability: Applicability,
    /// Replacements as `(span, replacement text)`.
    pub replacements: Vec<(Span, String)>,
}

/// A fully structured diagnostic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub(crate) level: Level,
    pub(crate) code: Option<DiagCode>,
    pub(crate) lint: Option<&'static str>,
    pub(crate) message: String,
    pub(crate) labels: Vec<Label>,
    pub(crate) notes: Vec<String>,
    pub(crate) helps: Vec<String>,
    pub(crate) suggestions: Vec<Suggestion>,
}

impl Diagnostic {
    /// Creates a diagnostic with a level and message.
    pub fn new(level: Level, message: impl Into<String>) -> Self {
        Self {
            level,
            code: None,
            lint: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    /// Attaches a stable code.
    #[must_use]
    pub fn with_code(mut self, code: DiagCode) -> Self {
        self.code = Some(code);
        self
    }

    /// Attaches the lint that owns this diagnostic.
    #[must_use]
    pub fn with_lint(mut self, lint: &'static str) -> Self {
        self.lint = Some(lint);
        self
    }

    /// Adds a label.
    #[must_use]
    pub fn with_label(mut self, span: Span, message: impl Into<String>, style: LabelStyle) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
            style,
        });
        self
    }

    /// Adds a note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Adds a help message.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.helps.push(help.into());
        self
    }

    /// Adds a suggestion.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: Suggestion) -> Self {
        self.suggestions.push(suggestion);
        self
    }

    /// Returns the severity.
    #[inline]
    pub fn level(&self) -> Level {
        self.level
    }

    /// Returns the code, if any.
    #[inline]
    pub fn code(&self) -> Option<DiagCode> {
        self.code
    }

    /// Returns the lint that owns this diagnostic, if any.
    #[inline]
    pub fn lint(&self) -> Option<&'static str> {
        self.lint
    }

    /// Returns the main message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the labels.
    #[inline]
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Returns the notes.
    #[inline]
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Returns the help messages.
    #[inline]
    pub fn helps(&self) -> &[String] {
        &self.helps
    }

    /// Returns the suggestions.
    #[inline]
    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }
}

#[cfg(test)]
mod tests {
    use super::{Applicability, Diagnostic, LabelStyle, Suggestion};
    use crate::code::DiagCode;
    use crate::level::Level;

    #[test]
    fn builder_collects_parts() {
        let map = seal_span::SourceMap::new();
        let id = map
            .add_file(seal_span::FileName::virtual_name("t"), "abc".to_owned())
            .expect("file registers");
        let span = map.span(id, 0..1).expect("valid span");
        let diagnostic = Diagnostic::new(Level::Error, "boom")
            .with_code(DiagCode::new("E0001"))
            .with_lint("unused_import")
            .with_label(span, "here", LabelStyle::Primary)
            .with_note("note")
            .with_help("help")
            .with_suggestion(Suggestion {
                message: "replace".to_owned(),
                applicability: Applicability::MachineApplicable,
                replacements: vec![(span, "x".to_owned())],
            });
        assert_eq!(diagnostic.level(), Level::Error);
        assert_eq!(diagnostic.code().map(DiagCode::as_str), Some("E0001"));
        assert_eq!(diagnostic.lint(), Some("unused_import"));
        assert_eq!(diagnostic.message(), "boom");
        assert_eq!(diagnostic.labels().len(), 1);
        assert_eq!(diagnostic.labels()[0].style, LabelStyle::Primary);
        assert_eq!(diagnostic.notes(), ["note"]);
        assert_eq!(diagnostic.helps(), ["help"]);
        assert_eq!(diagnostic.suggestions().len(), 1);
    }
}
