//! Diagnostics and values stored together by query-backed phases.
//!
//! A query database only memoizes values. Compiler phases also emit structured
//! diagnostics as side effects, so a cache hit would otherwise silently lose
//! warnings and errors. Every cached phase runs against a recording context and
//! stores the diagnostics with its value; replaying them through the session
//! preserves error counts, deduplication, and the selected output format.

use std::io;
use std::sync::{Arc, Mutex};

use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_session::Session;
use seal_span::SourceMap;

/// A cached value together with the diagnostics its computation emitted.
pub(crate) struct Cached<T> {
    /// The phase result.
    pub(crate) value: T,
    /// Diagnostics emitted while producing the value.
    pub(crate) diagnostics: Vec<Diagnostic>,
}

impl<T> Cached<T> {
    /// Creates a cached value.
    pub(crate) const fn new(value: T, diagnostics: Vec<Diagnostic>) -> Self {
        Self { value, diagnostics }
    }
}

/// Collects diagnostics in memory for one query computation.
#[derive(Clone, Default)]
struct RecordingEmitter {
    diagnostics: Arc<Mutex<Vec<Diagnostic>>>,
}

impl RecordingEmitter {
    fn take(&self) -> Vec<Diagnostic> {
        std::mem::take(
            &mut *self
                .diagnostics
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

impl Emitter for RecordingEmitter {
    fn emit(&mut self, diagnostic: &Diagnostic, _source_map: &SourceMap) -> io::Result<()> {
        self.diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(diagnostic.clone());
        Ok(())
    }
}

/// Runs `compute` against a private diagnostic context.
///
/// The private context has the same source map as the session, so spans render
/// identically, but it cannot alter the session's counts until the caller
/// replays the returned diagnostics.
pub(crate) fn with_diagnostics<T>(
    session: &Session,
    compute: impl FnOnce(&DiagCtxt) -> T,
) -> (T, Vec<Diagnostic>) {
    let emitter = RecordingEmitter::default();
    let dcx = DiagCtxt::new(Box::new(emitter.clone()), Arc::clone(session.source_map()));
    let value = compute(&dcx);
    (value, emitter.take())
}

/// Replays a cached phase's diagnostics into the session.
pub(crate) fn replay<T>(session: &Session, cached: &Cached<T>) {
    session.diag().replay(&cached.diagnostics);
}
