//! Deterministic symbol names.
//!
//! Seal functions are linked with external visibility so debuggers and other
//! objects can see them, but their symbols are prefixed to keep them out of the
//! C namespace. The C entry point is emitted separately by the backend.
//!
//! A generic instance appends a stable fingerprint of its full type arguments,
//! so two instantiations of one definition never share a symbol and the symbol
//! never depends on a display name or on arena handles.

use seal_def::DefId;
use seal_hash::Fingerprint;
use seal_symbol::Interner;

/// The C entry point symbol.
pub(crate) const ENTRY_POINT: &str = "main";

/// Returns the linker symbol for the Seal function `name`.
///
/// An impl function includes its parent type, so `Point::new` and `Line::new`
/// receive distinct symbols while free functions keep the short form.
pub(crate) fn function(name: &str, parent: Option<&str>) -> String {
    match parent {
        Some(parent) => format!("seal.{parent}.{name}"),
        None => format!("seal.{name}"),
    }
}

/// Returns the linker symbol for a generic instance.
///
/// `base` is the symbol of the definition, exactly as non-generic code names
/// it. The fingerprint identifies the definition and its complete type
/// arguments, so symbols stay stable across runs and stay distinct for distinct
/// arguments.
pub(crate) fn instance(base: &str, fingerprint: Fingerprint) -> String {
    format!("{base}.{fingerprint}")
}

/// Returns the linker symbol for a capture-free closure trampoline.
///
/// The parent names the item that owns the closure. The fingerprint identifies
/// the lifted body and its concrete type arguments, so nested closures and
/// generic instantiations never share a trampoline.
pub(crate) fn closure(parent: &str, fingerprint: Fingerprint) -> String {
    format!("seal.closure.{}.{fingerprint}", sanitize(parent))
}

/// Replaces characters that are not valid in a linker symbol with `_`.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// Returns the linker symbol for the C-ABI callback wrapper of one function.
///
/// `instance` identifies the Seal function and `signature` its concrete C
/// signature. Both are stable fingerprints, so a repeated cast reuses the same
/// wrapper and distinct signatures never share one.
pub(crate) fn callback(instance: Fingerprint, signature: Fingerprint) -> String {
    format!("seal.callback.{instance}.{signature}")
}

/// Returns the linker symbol for the drop glue of one concrete type.
///
/// The fingerprint covers the whole type structure, so distinct types never
/// share a glue and repeated builds emit the same symbol.
pub(crate) fn drop_glue(fingerprint: Fingerprint) -> String {
    format!("seal.drop.glue.{fingerprint}")
}

/// Returns the linker symbol for one vtable.
///
/// The fingerprint covers the concrete pointee, the principal trait, and its
/// instantiated arguments, so repeated builds emit the same name and distinct
/// trait objects cannot collide.
pub(crate) fn vtable(fingerprint: Fingerprint) -> String {
    format!("seal.vtable.{fingerprint}")
}

/// Returns a unique IR name for a local slot.
pub(crate) fn local(name: Option<&str>, index: usize) -> String {
    match name {
        Some(name) => format!("{name}.{index}"),
        None => format!("tmp.{index}"),
    }
}

/// Returns the linker symbol for the static `name`.
///
/// A file-scope static keeps its short form. A static declared inside a
/// function or an impl includes its parent, so two local statics with the same
/// spelling never share storage.
pub(crate) fn static_item(name: &str, parent: Option<&str>) -> String {
    qualify("static", name, parent)
}

/// Returns the linker symbol for the addressed constant `name`.
pub(crate) fn constant(name: &str, parent: Option<&str>) -> String {
    qualify("const", name, parent)
}

/// Returns the linker symbol for the value storage of the constant `name`.
///
/// Reading a constant by value loads a private copy, which is distinct from the
/// allocation the constant owns when it is also addressed.
pub(crate) fn value(name: &str, parent: Option<&str>) -> String {
    qualify("value", name, parent)
}

/// Returns the linker symbol for a promoted temporary.
pub(crate) fn promoted(owner: &str, index: u32) -> String {
    format!("seal.promoted.{owner}.{index}")
}

/// Returns the linker symbol for a promoted local of a `const func` frame.
pub(crate) fn const_fn_local(fingerprint: Fingerprint, index: u32) -> String {
    format!("seal.constfn.{fingerprint}.{index}")
}

fn qualify(kind: &str, name: &str, parent: Option<&str>) -> String {
    match parent {
        Some(parent) => format!("seal.{kind}.{parent}.{name}"),
        None => format!("seal.{kind}.{name}"),
    }
}

/// Returns the declared name of `def` and the dotted path of its parents.
///
/// A local value sits inside a function that may itself sit inside an impl, so
/// the whole chain is joined. That keeps two local statics with the same
/// spelling in different methods on different types from sharing storage.
pub(crate) fn definition(
    hir: &seal_hir::Hir,
    interner: &Interner,
    def: DefId,
) -> (String, Option<String>) {
    let name = interner.resolve(hir.definitions().get(def).name).to_owned();
    let mut components = Vec::new();
    let mut parent = hir.definitions().get(def).parent;
    while let Some(current) = parent {
        components.push(
            interner
                .resolve(hir.definitions().get(current).name)
                .to_owned(),
        );
        parent = hir.definitions().get(current).parent;
    }
    components.reverse();
    let parent = if components.is_empty() {
        None
    } else {
        Some(components.join("."))
    };
    (name, parent)
}
