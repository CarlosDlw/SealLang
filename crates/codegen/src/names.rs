//! Deterministic symbol names.
//!
//! Seal functions are linked with external visibility so debuggers and other
//! objects can see them, but their symbols are prefixed to keep them out of the
//! C namespace. The C entry point is emitted separately by the backend.

/// The C entry point symbol.
pub(crate) const ENTRY_POINT: &str = "main";

/// Returns the linker symbol for the Seal function `name`.
pub(crate) fn function(name: &str) -> String {
    format!("seal.{name}")
}

/// Returns a unique IR name for a local slot.
pub(crate) fn local(name: Option<&str>, index: usize) -> String {
    match name {
        Some(name) => format!("{name}.{index}"),
        None => format!("tmp.{index}"),
    }
}
