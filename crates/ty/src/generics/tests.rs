//! Tests for semantic generic parameters, substitutions, and instantiation.

use seal_def::DefId;
use seal_symbol::Interner;

use crate::generics::{Bound, ParamEnv};
use crate::{AdtKind, FieldDef, InferCtxt, ParamTyId, TyCtxt, TyId, TyKind, UnifyError};

fn def(index: u32) -> DefId {
    DefId::from_u32(index).expect("test definitions are small")
}

fn push_param(
    ctxt: &TyCtxt,
    interner: &Interner,
    name: &str,
    owner: DefId,
    index: u32,
) -> (TyId, ParamTyId) {
    let param = ctxt.push_param(def(100 + index), owner, index, interner.intern(name));
    (ctxt.param_ty(param), param)
}

fn pair(ctxt: &TyCtxt, interner: &Interner) -> (crate::AdtId, TyId, TyId) {
    let owner = def(0);
    let adt = ctxt.push_adt(owner, interner.intern("Pair"), AdtKind::Struct);
    ctxt.set_generic_count(adt, 2);
    let (t, _) = push_param(ctxt, interner, "T", owner, 0);
    let (u, _) = push_param(ctxt, interner, "U", owner, 1);
    ctxt.fill_adt(
        adt,
        vec![
            FieldDef {
                def: def(1),
                name: Some(interner.intern("first")),
                ty: t,
            },
            FieldDef {
                def: def(2),
                name: Some(interner.intern("second")),
                ty: u,
            },
        ],
    );
    (adt, t, u)
}

#[test]
fn params_are_interned_by_declaration() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let owner = def(0);
    let name = interner.intern("T");

    let first = ctxt.push_param(def(10), owner, 0, name);
    let second = ctxt.push_param(def(10), owner, 0, name);
    let other_scope = ctxt.push_param(def(10), def(1), 0, name);

    assert_eq!(first, second);
    assert_ne!(first, other_scope);
    assert_eq!(ctxt.param_len(), 2);

    let data = ctxt.param_data(first);
    assert_eq!(data.def, def(10));
    assert_eq!(data.owner, owner);
    assert_eq!(data.index, 0);
    assert_eq!(data.name, name);
    assert_eq!(ctxt.kind(ctxt.param_ty(first)), TyKind::Param(first));
}

#[test]
fn substs_are_structural_and_order_sensitive() {
    let ctxt = TyCtxt::new();
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();

    let empty = ctxt.substs(&[]);
    assert_eq!(empty, ctxt.empty_substs());
    assert_eq!(ctxt.substs_len(empty), 0);
    assert_eq!(ctxt.substs_len_total(), 1);

    let first = ctxt.substs(&[int, bool_ty]);
    let second = ctxt.substs(&[int, bool_ty]);
    let reversed = ctxt.substs(&[bool_ty, int]);

    assert_eq!(first, second);
    assert_ne!(first, reversed);
    assert_eq!(ctxt.substs_len(first), 2);
    assert_eq!(ctxt.substs_args(first), vec![int, bool_ty]);
    assert_eq!(ctxt.substs_len_total(), 3);
}

#[test]
fn adt_instantiation_is_structural_and_keeps_empty_substs_canonical() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let empty_adt = ctxt.push_adt(def(0), interner.intern("Empty"), AdtKind::Struct);
    ctxt.fill_adt(empty_adt, Vec::new());
    assert_eq!(
        ctxt.adt_ty(empty_adt),
        ctxt.adt_ty_with(empty_adt, ctxt.empty_substs())
    );

    let (pair, _, _) = pair(&ctxt, &interner);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();
    let first = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));
    let second = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));
    let reversed = ctxt.adt_ty_with(pair, ctxt.substs(&[bool_ty, int]));

    assert_eq!(first, second);
    assert_ne!(first, reversed);
    assert_eq!(
        ctxt.kind(first),
        TyKind::Adt {
            adt: pair,
            args: ctxt.substs(&[int, bool_ty]),
        }
    );
}

#[test]
#[should_panic(expected = "expects 2 type arguments, but received 0")]
fn adt_instantiation_rejects_the_wrong_arity() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, _, _) = pair(&ctxt, &interner);
    let _ = ctxt.adt_ty_with(pair, ctxt.empty_substs());
}

#[test]
fn substitute_is_identity_on_concrete_types() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, _, _) = pair(&ctxt, &interner);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();
    let concrete_pair = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));
    let substs = ctxt.substs(&[bool_ty, int]);

    let types = [
        int,
        bool_ty,
        ctxt.unit_ty(),
        ctxt.never_ty(),
        ctxt.error_ty(),
        ctxt.ref_ty(int),
        ctxt.mut_ref_ty(int),
        ctxt.array_ty(int, 3),
        ctxt.slice_ty(int),
        ctxt.tuple_ty(&[int, bool_ty]),
        ctxt.tuple_ty(&[]),
        concrete_pair,
    ];
    for ty in types {
        assert_eq!(ctxt.substitute(ty, substs), ty, "{ty:?} must be unchanged");
    }
}

#[test]
fn substitute_rewrites_every_structural_type() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, t, u) = pair(&ctxt, &interner);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();

    let pair_ut = ctxt.adt_ty_with(pair, ctxt.substs(&[u, t]));
    let nested = ctxt.adt_ty_with(pair, ctxt.substs(&[pair_ut, t]));
    let ty = ctxt.tuple_ty(&[
        t,
        ctxt.ref_ty(ctxt.mut_ref_ty(u)),
        ctxt.array_ty(u, 3),
        ctxt.slice_ty(t),
        pair_ut,
        nested,
    ]);
    let substs = ctxt.substs(&[int, bool_ty]);
    let substituted = ctxt.substitute(ty, substs);

    let pair_bi = ctxt.adt_ty_with(pair, ctxt.substs(&[bool_ty, int]));
    let nested_bi = ctxt.adt_ty_with(pair, ctxt.substs(&[pair_bi, int]));
    let expected = ctxt.tuple_ty(&[
        int,
        ctxt.ref_ty(ctxt.mut_ref_ty(bool_ty)),
        ctxt.array_ty(bool_ty, 3),
        ctxt.slice_ty(int),
        pair_bi,
        nested_bi,
    ]);

    assert_eq!(substituted, expected);
    assert_eq!(ctxt.substitute(substituted, substs), substituted);
}

#[test]
fn free_params_are_deduplicated_in_first_occurrence_order() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, t, u) = pair(&ctxt, &interner);
    let pair_ut = ctxt.adt_ty_with(pair, ctxt.substs(&[u, t]));
    let ty = ctxt.tuple_ty(&[t, pair_ut, t, u, ctxt.int_ty(crate::IntTy::I32)]);

    let params = ctxt.free_params(ty);
    assert_eq!(params.len(), 2);
    assert_eq!(ctxt.param_data(params[0]).name, interner.intern("T"));
    assert_eq!(ctxt.param_data(params[1]).name, interner.intern("U"));
    assert!(ctxt.contains_param(ty));
    assert!(!ctxt.contains_param(ctxt.int_ty(crate::IntTy::I32)));
    assert!(!ctxt.contains_param(ctxt.tuple_ty(&[ctxt.int_ty(crate::IntTy::I32), ctxt.bool_ty()])));
}

#[test]
fn parameter_queries_are_conservative_without_bounds() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (param_ty, param) = push_param(&ctxt, &interner, "T", def(0), 0);

    assert!(!ctxt.is_copy(param_ty));
    assert!(ctxt.is_sized(param_ty));
    assert!(!ctxt.needs_drop(param_ty));
    assert!(ctxt.contains_reference(param_ty));
    assert!(ctxt.contains_exclusive_reference(param_ty));

    let mut env = ParamEnv::new();
    env.insert(param, vec![Bound::Copy]);
    assert!(env.has_copy(param));
    assert!(ctxt.is_copy_with(param_ty, &env));
    assert!(ctxt.is_copy_with(ctxt.bool_ty(), &env));
}

#[test]
fn adt_queries_instantiate_every_field() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, t, u) = pair(&ctxt, &interner);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();
    let pair_ty = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));

    assert_eq!(
        ctxt.field_ty(pair, ctxt.substs(&[int, bool_ty]), def(1)),
        Some(int)
    );
    assert_eq!(
        ctxt.field_by_name_ty(
            pair,
            ctxt.substs(&[int, bool_ty]),
            interner.intern("second")
        ),
        Some(bool_ty)
    );
    assert!(ctxt.is_sized(pair_ty));
    assert!(!ctxt.is_copy(pair_ty));
    assert!(!ctxt.needs_drop(pair_ty));
    assert!(!ctxt.contains_reference(pair_ty));
    assert!(!ctxt.contains_exclusive_reference(pair_ty));

    let references = ctxt.adt_ty_with(pair, ctxt.substs(&[ctxt.ref_ty(int), ctxt.mut_ref_ty(int)]));
    assert!(ctxt.contains_reference(references));
    assert!(ctxt.contains_exclusive_reference(references));
    let nested_references = ctxt.adt_ty_with(pair, ctxt.substs(&[references, int]));
    assert!(ctxt.contains_reference(nested_references));
    assert!(ctxt.contains_exclusive_reference(nested_references));

    let pair_param = ctxt.adt_ty_with(pair, ctxt.substs(&[t, u]));
    assert_eq!(ctxt.field_ty(pair, ctxt.substs(&[t, u]), def(2)), Some(u));
    assert!(ctxt.contains_param(pair_param));
    let nested_param = ctxt.adt_ty_with(pair, ctxt.substs(&[pair_param, t]));
    assert!(ctxt.contains_param(nested_param));
}

#[test]
fn generic_unification_binds_arguments_and_nested_arguments() {
    let mut ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, _, _) = pair(&ctxt, &interner);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();

    let mut infer = InferCtxt::new();
    let first = infer.fresh_ty_var(&mut ctxt);
    let second = infer.fresh_ty_var(&mut ctxt);
    let inferred = ctxt.adt_ty_with(pair, ctxt.substs(&[first, second]));
    let concrete = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));
    infer
        .unify(&ctxt, inferred, concrete)
        .expect("arguments unify");
    assert_eq!(infer.resolve_shallow(&ctxt, first), int);
    assert_eq!(infer.resolve_shallow(&ctxt, second), bool_ty);

    let mut nested_infer = InferCtxt::new();
    let inner_first = nested_infer.fresh_ty_var(&mut ctxt);
    let inner_second = nested_infer.fresh_ty_var(&mut ctxt);
    let outer = nested_infer.fresh_ty_var(&mut ctxt);
    let inner = ctxt.adt_ty_with(pair, ctxt.substs(&[inner_first, inner_second]));
    let inferred = ctxt.adt_ty_with(pair, ctxt.substs(&[inner, outer]));
    let concrete_inner = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));
    let concrete = ctxt.adt_ty_with(pair, ctxt.substs(&[concrete_inner, int]));
    nested_infer
        .unify(&ctxt, inferred, concrete)
        .expect("nested arguments unify");
    assert_eq!(nested_infer.resolve_shallow(&ctxt, inner_first), int);
    assert_eq!(nested_infer.resolve_shallow(&ctxt, inner_second), bool_ty);
    assert_eq!(nested_infer.resolve_shallow(&ctxt, outer), int);
}

#[test]
fn rigid_params_do_not_unify_with_other_types() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (t, _) = push_param(&ctxt, &interner, "T", def(0), 0);
    let (u, _) = push_param(&ctxt, &interner, "U", def(0), 1);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let mut infer = InferCtxt::new();

    assert_eq!(
        infer.unify(&ctxt, t, int),
        Err(UnifyError::Mismatch {
            expected: t,
            found: int
        })
    );
    assert_eq!(
        infer.unify(&ctxt, t, u),
        Err(UnifyError::Mismatch {
            expected: t,
            found: u
        })
    );
    assert_eq!(infer.unify(&ctxt, t, t), Ok(()));
}

#[test]
fn a_mismatched_outer_adt_does_not_unify() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, _, _) = pair(&ctxt, &interner);
    let other = ctxt.push_adt(def(20), interner.intern("Other"), AdtKind::Struct);
    ctxt.set_generic_count(other, 2);
    ctxt.fill_adt(other, Vec::new());
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();
    let pair_ty = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));
    let other_ty = ctxt.adt_ty_with(other, ctxt.substs(&[int, bool_ty]));
    let mut infer = InferCtxt::new();
    assert_eq!(
        infer.unify(&ctxt, pair_ty, other_ty),
        Err(UnifyError::Mismatch {
            expected: pair_ty,
            found: other_ty
        })
    );
}

#[test]
fn occurs_check_descends_into_adt_arguments() {
    let mut ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, _, _) = pair(&ctxt, &interner);
    let mut infer = InferCtxt::new();
    let var = infer.fresh_ty_var(&mut ctxt);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let recursive = ctxt.adt_ty_with(pair, ctxt.substs(&[var, int]));

    assert_eq!(
        infer.unify(&ctxt, var, recursive),
        Err(UnifyError::Occurs {
            variable: match ctxt.kind(var) {
                TyKind::Infer(variable) => variable,
                other => panic!("expected an inference variable, found {other:?}"),
            },
            ty: recursive,
        })
    );
}

#[test]
fn deep_resolution_rebuilds_adt_arguments_and_preserves_unchanged_identity() {
    let mut ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, _, _) = pair(&ctxt, &interner);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();

    let concrete = ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty]));
    let mut infer = InferCtxt::new();
    assert_eq!(infer.resolve_deep(&mut ctxt, concrete), concrete);

    let var = infer.fresh_ty_var(&mut ctxt);
    let inferred = ctxt.adt_ty_with(pair, ctxt.substs(&[var, bool_ty]));
    infer.unify(&ctxt, var, int).expect("the argument binds");
    assert_eq!(infer.resolve_deep(&mut ctxt, inferred), concrete);
}

#[test]
fn display_renders_params_and_instantiated_adts() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (pair, t, u) = pair(&ctxt, &interner);
    let int = ctxt.int_ty(crate::IntTy::I32);
    let bool_ty = ctxt.bool_ty();

    assert_eq!(ctxt.display_with(t, &interner).to_string(), "T");
    assert_eq!(
        ctxt.display_with(ctxt.adt_ty_with(pair, ctxt.substs(&[t, u])), &interner)
            .to_string(),
        "Pair<T, U>"
    );
    assert_eq!(
        ctxt.display_with(
            ctxt.adt_ty_with(pair, ctxt.substs(&[int, bool_ty])),
            &interner
        )
        .to_string(),
        "Pair<i32, bool>"
    );
    let nested = ctxt.adt_ty_with(
        pair,
        ctxt.substs(&[ctxt.adt_ty_with(pair, ctxt.substs(&[t, u])), t]),
    );
    assert_eq!(
        ctxt.display_with(nested, &interner).to_string(),
        "Pair<Pair<T, U>, T>"
    );

    let self_param = ctxt.push_param(def(30), def(31), 0, interner.intern("Self"));
    assert_eq!(
        ctxt.display_with(ctxt.param_ty(self_param), &interner)
            .to_string(),
        "Self"
    );
}
