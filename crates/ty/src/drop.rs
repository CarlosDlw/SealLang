//! The destructor registry.
//!
//! An aggregate declares its destructor once, through `impl Drop for T`, and the
//! implementation covers the whole declaration: there is no per-instantiation
//! destructor. The registry therefore keys destructors by [`AdtId`] rather than
//! by instantiation.
//!
//! [`TyCtxt::needs_drop`](crate::TyCtxt::needs_drop) consults this registry
//! before it inspects the fields, because a declared destructor answers for the
//! type on its own. Lowering fills the registry after it validates every
//! `impl Drop`, so a missing entry always means "no destructor", never "not
//! lowered yet".

use seal_def::DefId;
use seal_hash::FxHashMap;

use crate::id::AdtId;

/// The destructor declared by each aggregate.
#[derive(Clone, Debug, Default)]
pub(crate) struct DropImpls {
    methods: FxHashMap<AdtId, DefId>,
}

impl DropImpls {
    /// Records `method` as the destructor of `adt`.
    ///
    /// Returns the destructor previously recorded for the same aggregate, so the
    /// caller can report a duplicate implementation instead of silently
    /// replacing one.
    pub(crate) fn insert(&mut self, adt: AdtId, method: DefId) -> Option<DefId> {
        self.methods.insert(adt, method)
    }

    /// Returns the destructor of `adt`, if the aggregate declares one.
    pub(crate) fn get(&self, adt: AdtId) -> Option<DefId> {
        self.methods.get(&adt).copied()
    }
}

#[cfg(test)]
mod tests {
    use seal_def::DefId;
    use seal_symbol::Interner;

    use super::DropImpls;
    use crate::{AdtKind, FieldDef, IntTy, TyCtxt, TyId, VariantDef};

    fn def(index: u32) -> DefId {
        DefId::from_u32(index).expect("test definitions are small")
    }

    /// Builds `struct Name { value: field }` and returns the aggregate and its
    /// type. Each test owns its context, so definition indices are local.
    fn struct_with(
        ctxt: &TyCtxt,
        interner: &Interner,
        name: &str,
        owner: u32,
        field: TyId,
    ) -> (crate::AdtId, TyId) {
        let adt = ctxt.push_adt(def(owner), interner.intern(name), AdtKind::Struct);
        ctxt.fill_adt(
            adt,
            vec![FieldDef {
                def: def(owner + 1),
                name: Some(interner.intern("value")),
                ty: field,
            }],
        );
        (adt, ctxt.adt_ty(adt))
    }

    #[test]
    fn inserting_a_destructor_returns_the_previous_method() {
        let mut impls = DropImpls::default();
        let adt = crate::AdtId::from_u32(0).expect("zero is representable");

        assert_eq!(impls.get(adt), None);
        assert_eq!(impls.insert(adt, def(10)), None);
        assert_eq!(impls.get(adt), Some(def(10)));
        assert_eq!(impls.insert(adt, def(11)), Some(def(10)));
        assert_eq!(impls.get(adt), Some(def(11)));
    }

    #[test]
    fn the_registry_keeps_aggregates_apart() {
        let mut impls = DropImpls::default();
        let first = crate::AdtId::from_u32(0).expect("zero is representable");
        let second = crate::AdtId::from_u32(1).expect("one is representable");

        assert_eq!(impls.insert(first, def(10)), None);
        assert_eq!(impls.insert(second, def(11)), None);
        assert_eq!(impls.get(first), Some(def(10)));
        assert_eq!(impls.get(second), Some(def(11)));
    }

    #[test]
    fn a_declared_destructor_makes_the_type_need_drop() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let int = ctxt.int_ty(IntTy::I32);
        let (adt, ty) = struct_with(&ctxt, &interner, "Buffer", 0, int);

        assert!(!ctxt.needs_drop(ty));
        assert_eq!(ctxt.set_drop_method(adt, def(9)), None);
        assert_eq!(ctxt.drop_method(adt), Some(def(9)));
        assert!(ctxt.needs_drop(ty));
    }

    #[test]
    fn a_type_without_destructors_and_trivial_fields_does_not_need_drop() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (_, ty) = struct_with(&ctxt, &interner, "Point", 0, ctxt.int_ty(IntTy::I32));

        assert!(!ctxt.needs_drop(ty));
        assert_eq!(
            ctxt.drop_method(crate::AdtId::from_u32(0).expect("small")),
            None
        );
    }

    #[test]
    fn a_field_with_a_destructor_makes_the_container_need_drop() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (inner_adt, inner) = struct_with(&ctxt, &interner, "Inner", 0, ctxt.bool_ty());
        assert_eq!(ctxt.set_drop_method(inner_adt, def(9)), None);
        let (_, outer) = struct_with(&ctxt, &interner, "Outer", 10, inner);

        assert!(ctxt.needs_drop(outer));
    }

    #[test]
    fn an_enum_needs_drop_when_any_variant_field_does() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (inner_adt, inner) = struct_with(&ctxt, &interner, "Inner", 0, ctxt.bool_ty());
        assert_eq!(ctxt.set_drop_method(inner_adt, def(9)), None);

        let adt = ctxt.push_adt(def(20), interner.intern("Shape"), AdtKind::Enum);
        ctxt.fill_variants(
            adt,
            vec![
                VariantDef {
                    def: def(21),
                    name: interner.intern("Empty"),
                    fields: Vec::new(),
                },
                VariantDef {
                    def: def(22),
                    name: interner.intern("Holds"),
                    fields: vec![FieldDef {
                        def: def(23),
                        name: None,
                        ty: inner,
                    }],
                },
            ],
        );

        assert!(ctxt.needs_drop(ctxt.adt_ty(adt)));
    }

    #[test]
    fn arrays_and_tuples_propagate_needs_drop() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (inner_adt, inner) = struct_with(&ctxt, &interner, "Inner", 0, ctxt.bool_ty());
        assert_eq!(ctxt.set_drop_method(inner_adt, def(9)), None);
        let int = ctxt.int_ty(IntTy::I32);

        assert!(ctxt.needs_drop(ctxt.array_ty(inner, 3)));
        assert!(ctxt.needs_drop(ctxt.tuple_ty(&[int, inner])));
        assert!(!ctxt.needs_drop(ctxt.array_ty(int, 3)));
        assert!(!ctxt.needs_drop(ctxt.tuple_ty(&[int, ctxt.bool_ty()])));
        assert!(!ctxt.needs_drop(ctxt.tuple_ty(&[])));
    }

    #[test]
    fn a_reference_cycle_does_not_need_drop() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (adt, ty) = struct_with(&ctxt, &interner, "Node", 0, ctxt.int_ty(IntTy::I32));
        ctxt.set_field_ty(adt, 0, ctxt.ref_ty(ty));

        assert!(!ctxt.needs_drop(ty));
    }

    #[test]
    fn a_by_value_cycle_terminates_without_needing_drop() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (adt, ty) = struct_with(&ctxt, &interner, "Loop", 0, ctxt.int_ty(IntTy::I32));
        // Type checking rejects an infinite-size type before this query runs;
        // the guard still has to terminate instead of recursing forever.
        ctxt.set_field_ty(adt, 0, ty);

        assert!(!ctxt.needs_drop(ty));
    }

    #[test]
    fn unsized_kinds_never_need_drop() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);

        assert!(!ctxt.needs_drop(ctxt.str_ty()));
        assert!(!ctxt.needs_drop(ctxt.slice_ty(int)));
        assert!(!ctxt.needs_drop(ctxt.dyn_ty(def(50), ctxt.substs(&[]))));
    }
}
