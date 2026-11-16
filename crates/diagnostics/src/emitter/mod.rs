//! Diagnostic output backends.

mod json;
mod rich;
mod silent;

use std::io;

use crate::diagnostic::Diagnostic;
use seal_span::SourceMap;

pub use json::{JsonEmitter, SCHEMA_VERSION};
pub use rich::{ColorChoice, RichEmitter, should_use_color};
pub use silent::SilentEmitter;

/// A destination for structured diagnostics.
///
/// Emitters must be deterministic for a given input: the same diagnostic and
/// source map produce the same bytes. They must not consult ambient state such as
/// the clock or environment variables; configuration is resolved by the caller.
pub trait Emitter: Send {
    /// Emits one diagnostic.
    fn emit(&mut self, diagnostic: &Diagnostic, source_map: &SourceMap) -> io::Result<()>;
}

impl Emitter for Box<dyn Emitter> {
    fn emit(&mut self, diagnostic: &Diagnostic, source_map: &SourceMap) -> io::Result<()> {
        (**self).emit(diagnostic, source_map)
    }
}
