//! The definition arena.

use seal_index::IndexVec;
use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::DefId;
use crate::kind::DefKind;

/// A named definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefData {
    /// What the definition introduces.
    pub kind: DefKind,
    /// The declared name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// The span of the whole definition.
    pub span: Span,
    /// The enclosing definition: the function for a parameter, and the owning
    /// function for a binding. `None` for functions.
    pub parent: Option<DefId>,
    /// Whether the definition may be assigned to.
    ///
    /// This is only meaningful for bindings: functions are never assignable, and
    /// parameters are immutable in the current language.
    pub mutable: bool,
}

/// The definitions of one file, in creation order.
#[derive(Clone, Debug, Default)]
pub struct Definitions {
    defs: IndexVec<DefId, DefData>,
}

impl Definitions {
    /// Creates an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a definition and returns its ID.
    pub fn push(&mut self, data: DefData) -> DefId {
        self.defs.push(data)
    }

    /// Returns a definition.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this table. IDs from another table are a
    /// programming error, not user input.
    pub fn get(&self, id: DefId) -> &DefData {
        &self.defs[id]
    }

    /// Returns the kind of `id`.
    pub fn kind(&self, id: DefId) -> DefKind {
        self.defs[id].kind
    }

    /// Returns the declared name of `id`.
    pub fn name(&self, id: DefId) -> Symbol {
        self.defs[id].name
    }

    /// Returns `true` if `id` may be assigned to.
    pub fn is_mutable(&self, id: DefId) -> bool {
        self.defs[id].mutable
    }

    /// Returns the number of definitions.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Returns `true` if the table holds no definitions.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Iterates over the definitions.
    pub fn iter(&self) -> impl Iterator<Item = &DefData> + '_ {
        self.defs.iter()
    }

    /// Iterates over `(id, definition)` pairs.
    pub fn iter_enumerated(&self) -> impl Iterator<Item = (DefId, &DefData)> + '_ {
        self.defs.iter_enumerated()
    }
}

#[cfg(test)]
mod tests {
    use seal_span::Span;
    use seal_symbol::Interner;

    use super::{DefData, Definitions};
    use crate::DefKind;

    fn definition(name: &Interner, text: &str, kind: DefKind, mutable: bool) -> DefData {
        DefData {
            kind,
            name: name.intern(text),
            name_span: Span::default(),
            span: Span::default(),
            parent: None,
            mutable,
        }
    }

    #[test]
    fn pushing_assigns_stable_ids() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        let first = definitions.push(definition(&interner, "first", DefKind::Function, false));
        let second = definitions.push(definition(&interner, "second", DefKind::Param, false));

        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(definitions.len(), 2);
        assert!(!definitions.is_empty());
        assert_eq!(definitions.kind(first), DefKind::Function);
        assert_eq!(definitions.name(second), interner.intern("second"));
    }

    #[test]
    fn mutation_only_holds_for_bindings() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        let function = definitions.push(definition(&interner, "f", DefKind::Function, false));
        let parameter = definitions.push(definition(&interner, "p", DefKind::Param, false));
        let binding = definitions.push(definition(&interner, "b", DefKind::Binding, true));

        assert!(!definitions.is_mutable(function));
        assert!(!definitions.is_mutable(parameter));
        assert!(definitions.is_mutable(binding));
    }

    #[test]
    fn iteration_is_in_creation_order() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        for name in ["a", "b", "c"] {
            definitions.push(definition(&interner, name, DefKind::Function, false));
        }
        let names: Vec<u32> = definitions
            .iter_enumerated()
            .map(|(id, _)| id.index())
            .collect();
        assert_eq!(names, [0, 1, 2]);
        assert_eq!(definitions.iter().count(), 3);
    }
}
