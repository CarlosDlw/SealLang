//! The scope stack used during lowering.
//!
//! Each rib is one scope. Lookup walks from the innermost rib outwards, so the
//! closest declaration wins and shadowing needs no special case.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_symbol::Symbol;

/// A stack of scopes, innermost last.
#[derive(Debug, Default)]
pub(crate) struct Scopes {
    ribs: Vec<FxHashMap<Symbol, DefId>>,
}

impl Scopes {
    /// Enters a new scope.
    pub(crate) fn push(&mut self) {
        self.ribs.push(FxHashMap::default());
    }

    /// Leaves the innermost scope.
    pub(crate) fn pop(&mut self) {
        self.ribs.pop();
    }

    /// Inserts a definition into the innermost scope, shadowing any previous
    /// definition of the same name in that scope.
    pub(crate) fn insert(&mut self, name: Symbol, def: DefId) {
        debug_assert!(!self.ribs.is_empty(), "insertion outside any scope");
        if let Some(rib) = self.ribs.last_mut() {
            rib.insert(name, def);
        }
    }

    /// Looks a name up in the innermost scope only, for duplicate detection.
    pub(crate) fn current(&self, name: Symbol) -> Option<DefId> {
        self.ribs.last().and_then(|rib| rib.get(&name).copied())
    }

    /// Looks a name up from the innermost scope outwards.
    pub(crate) fn lookup(&self, name: Symbol) -> Option<DefId> {
        self.ribs
            .iter()
            .rev()
            .find_map(|rib| rib.get(&name).copied())
    }
}

#[cfg(test)]
mod tests {
    use seal_def::{DefData, DefKind, Definitions};
    use seal_span::Span;
    use seal_symbol::Interner;

    use super::Scopes;

    fn define(definitions: &mut Definitions, name: seal_symbol::Symbol) -> seal_def::DefId {
        definitions.push(DefData::new(
            DefKind::Binding,
            name,
            Span::default(),
            Span::default(),
            None,
            false,
        ))
    }

    #[test]
    fn inner_scopes_shadow_outer_ones() {
        let interner = Interner::new();
        let name = interner.intern("x");
        let mut definitions = Definitions::new();
        let outer = define(&mut definitions, name);
        let inner = define(&mut definitions, name);

        let mut scopes = Scopes::default();
        scopes.push();
        scopes.insert(name, outer);
        scopes.push();
        scopes.insert(name, inner);

        assert_eq!(scopes.lookup(name), Some(inner));
        assert_eq!(scopes.current(name), Some(inner));

        scopes.pop();
        assert_eq!(scopes.lookup(name), Some(outer));
        scopes.pop();
        assert_eq!(scopes.lookup(name), None);
    }

    #[test]
    fn same_scope_insertion_replaces_the_definition() {
        let interner = Interner::new();
        let name = interner.intern("shadowed");
        let mut definitions = Definitions::new();
        let first = define(&mut definitions, name);
        let second = define(&mut definitions, name);

        let mut scopes = Scopes::default();
        scopes.push();
        scopes.insert(name, first);
        scopes.insert(name, second);

        assert_eq!(scopes.current(name), Some(second));
        scopes.pop();
    }
}
