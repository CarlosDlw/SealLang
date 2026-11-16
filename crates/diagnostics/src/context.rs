//! The diagnostic context: emission, deduplication, counts, and guarantees.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::code::DiagCode;
use crate::diagnostic::{Applicability, Diagnostic, LabelStyle, Suggestion};
use crate::emitter::Emitter;
use crate::level::Level;
use seal_span::{SourceMap, Span};

/// Proof that an error was emitted.
///
/// Only [`DiagCtxt`] constructs this type, and functions that can continue after
/// an error require it. That makes "emit an error and keep going as if nothing
/// happened" a compile error rather than a review comment.
#[derive(Clone, Copy, Debug)]
pub struct ErrorGuaranteed(());

/// A fatal error that prevented compilation from starting.
#[derive(Debug)]
pub struct FatalError {
    message: String,
}

impl FatalError {
    /// Returns the message that was already emitted.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for FatalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FatalError {}

/// Payload used to abort on an internal compiler error.
#[derive(Debug)]
pub struct IcePanic;

/// Default maximum number of errors before emission stops.
pub const DEFAULT_ERROR_LIMIT: usize = 100;

struct Inner {
    emitter: Box<dyn Emitter>,
    source_map: Arc<SourceMap>,
    seen: BTreeSet<(Level, Option<DiagCode>, Span, String)>,
    error_count: usize,
    warning_count: usize,
    error_limit: usize,
    first_error: Option<ErrorGuaranteed>,
    aborted: bool,
    emitter_error: Option<io::Error>,
}

/// Owns the diagnostic emitter and tracks the compilation's error state.
pub struct DiagCtxt {
    inner: Mutex<Inner>,
}

impl fmt::Debug for DiagCtxt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiagCtxt").finish_non_exhaustive()
    }
}

impl DiagCtxt {
    /// Creates a diagnostic context.
    pub fn new(emitter: Box<dyn Emitter>, source_map: Arc<SourceMap>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                emitter,
                source_map,
                seen: BTreeSet::new(),
                error_count: 0,
                warning_count: 0,
                error_limit: DEFAULT_ERROR_LIMIT,
                first_error: None,
                aborted: false,
                emitter_error: None,
            }),
        }
    }

    /// Sets the number of errors after which emission stops.
    pub fn set_error_limit(&self, limit: usize) {
        self.lock().error_limit = limit.max(1);
    }

    /// Returns the number of distinct errors emitted.
    pub fn error_count(&self) -> usize {
        self.lock().error_count
    }

    /// Returns the number of distinct warnings emitted.
    pub fn warning_count(&self) -> usize {
        self.lock().warning_count
    }

    /// Returns `true` if at least one error was emitted.
    pub fn has_errors(&self) -> bool {
        self.lock().first_error.is_some()
    }

    /// Returns the first error guarantee, if any.
    pub fn error_guarantee(&self) -> Option<ErrorGuaranteed> {
        self.lock().first_error
    }

    /// Returns the emitter I/O error, if writing a diagnostic failed.
    ///
    /// Callers must surface this (usually by exiting non-zero): a compiler that
    /// cannot write diagnostics must not report success.
    pub fn emitter_error(&self) -> Option<io::Error> {
        self.lock().emitter_error.take()
    }

    /// Returns `Ok(())` if no error was emitted, or the error guarantee.
    pub fn abort_if_errors(&self) -> Result<(), ErrorGuaranteed> {
        match self.lock().first_error {
            Some(guarantee) => Err(guarantee),
            None => Ok(()),
        }
    }

    /// Creates an error builder.
    pub fn error(&self, message: impl Into<String>) -> ErrorBuilder<'_> {
        ErrorBuilder(self.builder(Level::Error, message))
    }

    /// Creates a warning builder.
    pub fn warning(&self, message: impl Into<String>) -> WarningBuilder<'_> {
        WarningBuilder(self.builder(Level::Warning, message))
    }

    /// Emits a standalone note.
    pub fn note(&self, message: impl Into<String>) {
        self.emit(Diagnostic::new(Level::Note, message));
    }

    /// Emits a standalone help message.
    pub fn help(&self, message: impl Into<String>) {
        self.emit(Diagnostic::new(Level::Help, message));
    }

    /// Emits a fatal diagnostic and returns the error that callers propagate.
    pub fn fatal(&self, message: impl Into<String>) -> FatalError {
        let message = message.into();
        self.emit(Diagnostic::new(Level::Fatal, message.clone()));
        FatalError { message }
    }

    /// Emits an internal compiler error and aborts the process.
    ///
    /// The report is flushed through the configured emitter before the panic, so
    /// even a JSON consumer sees a structured bug report.
    pub fn bug(&self, message: impl Into<String>) -> ! {
        self.emit(Diagnostic::new(Level::Bug, message));
        std::panic::panic_any(IcePanic)
    }

    /// Emits an internal compiler error at `span` and aborts the process.
    pub fn span_bug(&self, span: Span, message: impl Into<String>) -> ! {
        self.emit(Diagnostic::new(Level::Bug, message).with_label(
            span,
            "internal compiler error here",
            LabelStyle::Primary,
        ));
        std::panic::panic_any(IcePanic)
    }

    /// Emits a diagnostic, deduplicating by `(code, first label span)`.
    ///
    /// Returns a guarantee for error-level diagnostics. Duplicate errors return
    /// the existing guarantee without emitting twice.
    pub fn emit(&self, diagnostic: Diagnostic) -> Option<ErrorGuaranteed> {
        let mut inner = self.lock();
        if inner.aborted {
            return inner.first_error;
        }

        // Deduplicate on everything the user sees, so two different errors at
        // the same (or absent) span are not collapsed. Span-less diagnostics use
        // `Span::default()` as their location key.
        let key = (
            diagnostic.level(),
            diagnostic.code(),
            diagnostic
                .labels()
                .first()
                .map_or(Span::default(), |label| label.span),
            diagnostic.message().to_owned(),
        );
        if !inner.seen.insert(key) {
            return inner.first_error;
        }

        if diagnostic.level() == Level::Error {
            if inner.error_count >= inner.error_limit {
                inner.aborted = true;
                let mut guard = inner;
                let synthetic =
                    Diagnostic::new(Level::Fatal, "too many errors emitted, stopping now");
                let source_map = Arc::clone(&guard.source_map);
                if let Err(error) = guard.emitter.emit(&synthetic, &source_map) {
                    guard.emitter_error = Some(error);
                }
                return guard.first_error;
            }
            inner.error_count += 1;
        } else if diagnostic.level() == Level::Warning {
            inner.warning_count += 1;
        }

        let source_map = Arc::clone(&inner.source_map);
        if let Err(error) = inner.emitter.emit(&diagnostic, &source_map) {
            if inner.emitter_error.is_none() {
                inner.emitter_error = Some(error);
            }
        }

        if diagnostic.level().is_error() {
            let guarantee = ErrorGuaranteed(());
            if inner.first_error.is_none() {
                inner.first_error = Some(guarantee);
            }
            Some(guarantee)
        } else {
            None
        }
    }

    fn builder(&self, level: Level, message: impl Into<String>) -> DiagnosticBuilder<'_> {
        DiagnosticBuilder {
            dcx: self,
            diagnostic: Diagnostic::new(level, message),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Shared builder operations for [`ErrorBuilder`] and [`WarningBuilder`].
///
/// Construct one with [`DiagCtxt::error`] or [`DiagCtxt::warning`].
pub struct DiagnosticBuilder<'a> {
    dcx: &'a DiagCtxt,
    diagnostic: Diagnostic,
}

impl DiagnosticBuilder<'_> {
    /// Attaches a stable code.
    #[must_use]
    pub fn code(mut self, code: DiagCode) -> Self {
        self.diagnostic = self.diagnostic.with_code(code);
        self
    }

    /// Adds a primary label.
    #[must_use]
    pub fn primary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.diagnostic = self
            .diagnostic
            .with_label(span, message, LabelStyle::Primary);
        self
    }

    /// Adds a secondary label.
    #[must_use]
    pub fn secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.diagnostic = self
            .diagnostic
            .with_label(span, message, LabelStyle::Secondary);
        self
    }

    /// Adds a note.
    #[must_use]
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.diagnostic = self.diagnostic.with_note(note);
        self
    }

    /// Adds a help message.
    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.diagnostic = self.diagnostic.with_help(help);
        self
    }

    /// Adds a machine-applicable suggestion.
    #[must_use]
    pub fn suggestion(
        mut self,
        message: impl Into<String>,
        replacements: Vec<(Span, String)>,
    ) -> Self {
        self.diagnostic = self.diagnostic.with_suggestion(Suggestion {
            message: message.into(),
            applicability: Applicability::MachineApplicable,
            replacements,
        });
        self
    }

    /// Adds a suggestion with an explicit applicability.
    #[must_use]
    pub fn suggestion_with_applicability(
        mut self,
        message: impl Into<String>,
        applicability: Applicability,
        replacements: Vec<(Span, String)>,
    ) -> Self {
        self.diagnostic = self.diagnostic.with_suggestion(Suggestion {
            message: message.into(),
            applicability,
            replacements,
        });
        self
    }

    /// Returns the assembled diagnostic without emitting it.
    pub fn into_diagnostic(self) -> Diagnostic {
        self.diagnostic
    }
}

/// Builder for error diagnostics. [`ErrorBuilder::emit`] returns the proof.
#[must_use = "call `emit` to report the diagnostic"]
pub struct ErrorBuilder<'a>(DiagnosticBuilder<'a>);

impl ErrorBuilder<'_> {
    /// Attaches a stable code.
    pub fn code(self, code: DiagCode) -> Self {
        Self(self.0.code(code))
    }

    /// Adds a primary label.
    pub fn primary(self, span: Span, message: impl Into<String>) -> Self {
        Self(self.0.primary(span, message))
    }

    /// Adds a secondary label.
    pub fn secondary(self, span: Span, message: impl Into<String>) -> Self {
        Self(self.0.secondary(span, message))
    }

    /// Adds a note.
    pub fn note(self, note: impl Into<String>) -> Self {
        Self(self.0.note(note))
    }

    /// Adds a help message.
    pub fn help(self, help: impl Into<String>) -> Self {
        Self(self.0.help(help))
    }

    /// Adds a machine-applicable suggestion.
    pub fn suggestion(self, message: impl Into<String>, replacements: Vec<(Span, String)>) -> Self {
        Self(self.0.suggestion(message, replacements))
    }

    /// Emits the error and returns the guarantee.
    pub fn emit(self) -> ErrorGuaranteed {
        self.0
            .dcx
            .emit(self.0.diagnostic)
            .expect("error-level diagnostics always produce a guarantee")
    }
}

/// Builder for warning diagnostics.
#[must_use = "call `emit` to report the diagnostic"]
pub struct WarningBuilder<'a>(DiagnosticBuilder<'a>);

impl WarningBuilder<'_> {
    /// Attaches a stable code.
    pub fn code(self, code: DiagCode) -> Self {
        Self(self.0.code(code))
    }

    /// Adds a primary label.
    pub fn primary(self, span: Span, message: impl Into<String>) -> Self {
        Self(self.0.primary(span, message))
    }

    /// Adds a secondary label.
    pub fn secondary(self, span: Span, message: impl Into<String>) -> Self {
        Self(self.0.secondary(span, message))
    }

    /// Adds a note.
    pub fn note(self, note: impl Into<String>) -> Self {
        Self(self.0.note(note))
    }

    /// Adds a help message.
    pub fn help(self, help: impl Into<String>) -> Self {
        Self(self.0.help(help))
    }

    /// Adds a machine-applicable suggestion.
    pub fn suggestion(self, message: impl Into<String>, replacements: Vec<(Span, String)>) -> Self {
        Self(self.0.suggestion(message, replacements))
    }

    /// Emits the warning.
    pub fn emit(self) {
        let _ = self.0.dcx.emit(self.0.diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{DEFAULT_ERROR_LIMIT, DiagCtxt, IcePanic};
    use crate::Diagnostic;
    use crate::emitter::SilentEmitter;
    use crate::level::Level;
    use seal_span::{FileName, SourceMap};

    fn context() -> (DiagCtxt, Arc<SourceMap>) {
        let map = Arc::new(SourceMap::new());
        let emitter = SilentEmitter::new();
        let dcx = DiagCtxt::new(Box::new(emitter), Arc::clone(&map));
        (dcx, map)
    }

    fn span(map: &SourceMap) -> seal_span::Span {
        let id = map
            .add_file(FileName::virtual_name("t"), "abc".to_owned())
            .expect("file registers");
        map.span(id, 0..1).expect("valid span")
    }

    #[test]
    fn error_returns_guarantee_and_sets_state() {
        let (dcx, map) = context();
        let guarantee = dcx.error("boom").primary(span(&map), "here").emit();
        assert!(dcx.has_errors());
        assert_eq!(dcx.error_count(), 1);
        assert!(dcx.abort_if_errors().is_err());
        let _ = guarantee;
    }

    #[test]
    fn warnings_do_not_count_as_errors() {
        let (dcx, map) = context();
        dcx.warning("careful").primary(span(&map), "here").emit();
        assert!(!dcx.has_errors());
        assert_eq!(dcx.warning_count(), 1);
        assert!(dcx.abort_if_errors().is_ok());
    }

    #[test]
    fn duplicate_errors_are_emitted_once() {
        let (dcx, map) = context();
        let span = span(&map);
        dcx.error("boom")
            .code(crate::DiagCode::new("E0001"))
            .primary(span, "here")
            .emit();
        dcx.error("boom")
            .code(crate::DiagCode::new("E0001"))
            .primary(span, "here")
            .emit();
        assert_eq!(dcx.error_count(), 1);
    }

    #[test]
    fn distinct_codes_at_the_same_span_are_not_duplicates() {
        let (dcx, map) = context();
        let span = span(&map);
        dcx.error("a")
            .code(crate::DiagCode::new("E0001"))
            .primary(span, "")
            .emit();
        dcx.error("b")
            .code(crate::DiagCode::new("E0002"))
            .primary(span, "")
            .emit();
        assert_eq!(dcx.error_count(), 2);
    }

    #[test]
    fn error_limit_stops_emission() {
        let (dcx, _map) = context();
        dcx.set_error_limit(2);
        for index in 0..10 {
            dcx.error(format!("error {index}")).emit();
        }
        assert_eq!(dcx.error_count(), 2);
        assert!(dcx.has_errors());
        const { assert!(DEFAULT_ERROR_LIMIT >= 2) };
    }

    #[test]
    fn fatal_returns_a_display_error() {
        let (dcx, _map) = context();
        let fatal = dcx.fatal("cannot continue");
        assert_eq!(fatal.message(), "cannot continue");
        assert_eq!(fatal.to_string(), "cannot continue");
        assert!(dcx.has_errors());
    }

    #[test]
    fn ice_aborts_with_marker() {
        let (dcx, _map) = context();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dcx.bug("broken invariant");
        }));
        let payload = result.expect_err("bug must panic");
        assert!(payload.downcast_ref::<IcePanic>().is_some());
    }

    #[test]
    fn emitter_failure_is_recorded() {
        struct Failing;
        impl std::io::Write for Failing {
            fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let map = Arc::new(SourceMap::new());
        let emitter = crate::RichEmitter::plain(Failing);
        let dcx = DiagCtxt::new(Box::new(emitter), map);
        dcx.error("boom").emit();
        assert!(dcx.emitter_error().is_some());
        assert!(dcx.emitter_error().is_none());
    }

    #[test]
    fn structured_builders_preserve_all_parts() {
        let (dcx, map) = context();
        let span = span(&map);
        let diagnostic = dcx
            .error("full")
            .code(crate::DiagCode::new("E0001"))
            .primary(span, "primary")
            .secondary(span, "secondary")
            .note("note")
            .help("help")
            .suggestion("fix", vec![(span, "x".to_owned())])
            .0
            .into_diagnostic();
        assert_eq!(diagnostic.level(), Level::Error);
        assert_eq!(diagnostic.labels().len(), 2);
        assert_eq!(diagnostic.notes().len(), 1);
        assert_eq!(diagnostic.helps().len(), 1);
        assert_eq!(diagnostic.suggestions().len(), 1);
        let _ = Diagnostic::new(Level::Note, "unused");
    }
}
