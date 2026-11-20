//! Generic parameters, their scopes, and implementation records.
//!
//! The semantic identities live in [`seal_ty`]; this module owns the HIR view
//! that lowering needs while a declaration is being processed. A scope stack
//! makes an impl's parameters visible to its methods while keeping two methods'
//! own parameters distinct.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_symbol::Symbol;
use seal_ty::{GenericParamDef, Generics, ParamTyId, TyId};

/// One implementation block with a nominal target.
///
/// Trait implementations are recorded as [`seal_ty::TraitImplDef`] in the type
/// context because they participate in trait lookup. Inherent implementations
/// have no trait ref, so they are recorded here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InherentImpl {
    /// The definition created for the impl block.
    pub def: DefId,
    /// The target type.
    pub self_ty: TyId,
    /// The impl's own generic parameters.
    pub generics: Generics,
    /// Function names bound to their definitions, in source order.
    pub methods: Vec<(Symbol, DefId)>,
}

/// A stack of generic-parameter ribs, innermost last.
#[derive(Debug, Default)]
pub(crate) struct GenericScopes {
    ribs: Vec<FxHashMap<Symbol, ParamTyId>>,
}

impl GenericScopes {
    /// Enters one owner's scope and records its parameters.
    pub(crate) fn push(&mut self, params: &[GenericParamDef]) {
        let mut rib = FxHashMap::default();
        for param in params {
            rib.insert(param.name, param.param);
        }
        self.ribs.push(rib);
    }

    /// Leaves the innermost owner's scope.
    pub(crate) fn pop(&mut self) {
        self.ribs.pop();
    }

    /// Returns the parameter named `name`, searching outward.
    pub(crate) fn lookup(&self, name: Symbol) -> Option<ParamTyId> {
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
    use seal_ty::{GenericParamDef, ParamTyId};

    use super::GenericScopes;

    fn param(
        defs: &mut Definitions,
        interner: &Interner,
        param: ParamTyId,
        name: &str,
        index: u32,
    ) -> GenericParamDef {
        let def = defs.push(DefData::new(
            DefKind::GenericParam,
            interner.intern(name),
            Span::default(),
            Span::default(),
            None,
            false,
        ));
        GenericParamDef::new(def, param, interner.intern(name), index)
    }

    #[test]
    fn inner_scopes_shadow_outer_parameters() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        let outer = param(
            &mut definitions,
            &interner,
            ParamTyId::from_u32(0).expect("small index"),
            "T",
            0,
        );
        let inner = param(
            &mut definitions,
            &interner,
            ParamTyId::from_u32(1).expect("small index"),
            "T",
            1,
        );

        let mut scopes = GenericScopes::default();
        scopes.push(std::slice::from_ref(&outer));
        scopes.push(std::slice::from_ref(&inner));
        assert_eq!(scopes.lookup(outer.name), Some(inner.param));

        scopes.pop();
        assert_eq!(scopes.lookup(outer.name), Some(outer.param));
        scopes.pop();
        assert_eq!(scopes.lookup(outer.name), None);
    }
}
