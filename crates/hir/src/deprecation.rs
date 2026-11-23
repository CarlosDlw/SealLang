//! Use-site deprecation warnings.
//!
//! The warning is emitted after name resolution has selected a definition, so
//! every surface that resolves a name reaches the same formatting and policy.
//! Type checking calls this function for method calls and field accesses,
//! which are resolved later than ordinary paths.

use seal_def::DefId;
use seal_diagnostics::{DEPRECATED, DiagCtxt};
use seal_span::Span;
use seal_symbol::Interner;

use crate::Hir;

/// Warns that `def` is used at `use_span`.
///
/// A use inside the deprecated declaration itself is suppressed: a deprecated
/// function may call itself, and a deprecated struct may refer to itself, without
/// producing a warning on every line of its own definition. The diagnostic is
/// attached to the `deprecated` lint so `--allow-lint` and `--deny-lint` can
/// control it. Repeated uses at the same span are deduplicated by the diagnostic
/// context.
pub fn warn_deprecated(hir: &Hir, dcx: &DiagCtxt, interner: &Interner, def: DefId, use_span: Span) {
    let data = hir.definitions().get(def);
    let Some(deprecation) = data.deprecated else {
        return;
    };
    if data.span.contains_span(use_span) {
        return;
    }

    let name = interner.resolve(data.name);
    let since = interner.resolve(deprecation.since);
    let mut warning = dcx
        .warning(format!("`{name}` is deprecated since {since}"))
        .lint(DEPRECATED)
        .primary(use_span, "this use is deprecated")
        .secondary(
            data.name_span,
            format!("the {} was deprecated here", data.kind.description()),
        );
    if let Some(note) = deprecation.note {
        warning = warning.note(interner.resolve(note));
    }
    if let Some(replacement) = deprecation.replacement {
        warning = warning.help(format!("use {}", interner.resolve(replacement)));
    }
    warning.emit();
}
