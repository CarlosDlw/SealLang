//! In-memory emitter used by tests and tools.

use std::io;
use std::sync::Mutex;

use crate::diagnostic::Diagnostic;
use crate::emitter::Emitter;
use seal_span::SourceMap;

/// Collects diagnostics instead of writing them anywhere.
#[derive(Default)]
pub struct SilentEmitter {
    diagnostics: Mutex<Vec<Diagnostic>>,
}

impl SilentEmitter {
    /// Creates an empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of the collected diagnostics.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Removes and returns the collected diagnostics.
    pub fn take(&self) -> Vec<Diagnostic> {
        std::mem::take(
            &mut *self
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }
}

impl Emitter for SilentEmitter {
    fn emit(&mut self, diagnostic: &Diagnostic, _source_map: &SourceMap) -> io::Result<()> {
        self.diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(diagnostic.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SilentEmitter;
    use crate::Diagnostic;
    use crate::emitter::Emitter;
    use crate::level::Level;

    #[test]
    fn collects_and_drains() {
        let mut emitter = SilentEmitter::new();
        let map = seal_span::SourceMap::new();
        emitter
            .emit(&Diagnostic::new(Level::Warning, "w"), &map)
            .expect("emit succeeds");
        assert_eq!(emitter.diagnostics().len(), 1);
        assert_eq!(emitter.take().len(), 1);
        assert!(emitter.diagnostics().is_empty());
    }
}
