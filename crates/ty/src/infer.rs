//! Inference variables and unification.
//!
//! A variable starts unbound. Unification binds it to another type, and
//! resolution follows those bindings. Binding is the only operation that can
//! make two distinct types equal; no coercion happens here, so `i32` and `u32`
//! never unify.

use std::fmt;

use seal_index::IndexVec;

use crate::ctxt::TyCtxt;
use crate::id::{InferTyId, TyId};
use crate::kind::TyKind;

/// Holds the inference variables created for one body.
#[derive(Clone, Debug, Default)]
pub struct InferCtxt {
    /// The bound type of each variable, or `None` while it is unbound.
    ///
    /// Following bindings always terminates because binding performs an occurs
    /// check first.
    vars: IndexVec<InferTyId, Option<TyId>>,
}

impl InferCtxt {
    /// Creates a table with no variables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a fresh, unbound type variable and returns the type standing for
    /// it.
    pub fn fresh_ty_var(&mut self, ctxt: &mut TyCtxt) -> TyId {
        let var = self.vars.push(None);
        ctxt.infer_ty(var)
    }

    /// Returns the number of variables created so far.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// Returns `true` if no variable has been created.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Follows bindings until it reaches an unbound variable or a concrete type,
    /// compressing the path on the way.
    pub fn resolve_shallow(&mut self, ctxt: &TyCtxt, ty: TyId) -> TyId {
        let root = self.root(ctxt, ty);
        let mut current = ty;
        while current != root {
            match ctxt.kind(current) {
                TyKind::Infer(var) => match self.vars[var] {
                    Some(next) => {
                        self.vars[var] = Some(root);
                        current = next;
                    }
                    // An unbound variable is the root, so `current == root` and
                    // the loop condition would have stopped already.
                    None => break,
                },
                _ => break,
            }
        }
        root
    }

    /// Unifies `expected` with `found`, binding variables as needed.
    ///
    /// Returns `Ok(())` when the types are already equal or can be made equal by
    /// binding variables. The error type reports the first conflict; callers stop
    /// at the first error rather than continuing with a partially bound table.
    pub fn unify(&mut self, ctxt: &TyCtxt, expected: TyId, found: TyId) -> Result<(), UnifyError> {
        let expected = self.resolve_shallow(ctxt, expected);
        let found = self.resolve_shallow(ctxt, found);
        if expected == found {
            return Ok(());
        }
        match (ctxt.kind(expected), ctxt.kind(found)) {
            // An error type already produced its diagnostic; absorbing the other
            // side prevents that single error from cascading.
            (TyKind::Error, _) | (_, TyKind::Error) => Ok(()),
            (TyKind::Infer(var), _) => self.bind(ctxt, var, found),
            (_, TyKind::Infer(var)) => self.bind(ctxt, var, expected),
            (
                TyKind::Ref {
                    kind: expected_kind,
                    pointee: expected,
                },
                TyKind::Ref {
                    kind: found_kind,
                    pointee: found,
                },
            ) if expected_kind == found_kind => self.unify(ctxt, expected, found),
            (
                TyKind::Array {
                    element: expected_element,
                    length: expected_length,
                },
                TyKind::Array {
                    element: found_element,
                    length: found_length,
                },
            ) if expected_length == found_length => {
                self.unify(ctxt, expected_element, found_element)
            }
            (
                TyKind::Slice {
                    element: expected_element,
                },
                TyKind::Slice {
                    element: found_element,
                },
            ) => self.unify(ctxt, expected_element, found_element),
            _ => Err(UnifyError::Mismatch { expected, found }),
        }
    }

    /// Replaces every bound variable inside `ty`, rebuilding pointer types when
    /// their pointee resolves to something else.
    ///
    /// Unbound variables stay as they are, so callers can report them as
    /// ambiguous.
    pub fn resolve_deep(&mut self, ctxt: &mut TyCtxt, ty: TyId) -> TyId {
        let shallow = self.resolve_shallow(ctxt, ty);
        match ctxt.kind(shallow) {
            TyKind::Ref { kind, pointee } => {
                let resolved = self.resolve_deep(ctxt, pointee);
                if resolved != pointee {
                    return ctxt.ref_ty_with(kind, resolved);
                }
            }
            TyKind::Array { element, length } => {
                let resolved = self.resolve_deep(ctxt, element);
                if resolved != element {
                    return ctxt.array_ty(resolved, length);
                }
            }
            TyKind::Slice { element } => {
                let resolved = self.resolve_deep(ctxt, element);
                if resolved != element {
                    return ctxt.slice_ty(resolved);
                }
            }
            _ => {}
        }
        shallow
    }

    /// Returns the final type reachable from `ty` by following bindings.
    fn root(&self, ctxt: &TyCtxt, ty: TyId) -> TyId {
        let mut current = ty;
        loop {
            match ctxt.kind(current) {
                TyKind::Infer(var) => match self.vars[var] {
                    Some(next) => current = next,
                    None => return current,
                },
                _ => return current,
            }
        }
    }

    /// Binds `var` to `ty` unless that would make the variable contain itself.
    fn bind(&mut self, ctxt: &TyCtxt, var: InferTyId, ty: TyId) -> Result<(), UnifyError> {
        let ty = self.resolve_shallow(ctxt, ty);
        if self.occurs(ctxt, var, ty) {
            return Err(UnifyError::Occurs { variable: var, ty });
        }
        self.vars[var] = Some(ty);
        Ok(())
    }

    /// Returns `true` when `var` appears inside `ty`.
    fn occurs(&mut self, ctxt: &TyCtxt, var: InferTyId, ty: TyId) -> bool {
        let ty = self.resolve_shallow(ctxt, ty);
        match ctxt.kind(ty) {
            TyKind::Infer(other) => other == var,
            TyKind::Ref { pointee, .. } => self.occurs(ctxt, var, pointee),
            TyKind::Array { element, .. } | TyKind::Slice { element } => {
                self.occurs(ctxt, var, element)
            }
            _ => false,
        }
    }
}

/// Why two types could not be made equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnifyError {
    /// The types are distinct and neither is an inference variable.
    Mismatch {
        /// The type the context required.
        expected: TyId,
        /// The type actually found.
        found: TyId,
    },
    /// An inference variable would have to contain itself, which would make the
    /// type infinite.
    Occurs {
        /// The variable that would become recursive.
        variable: InferTyId,
        /// The type that mentions it.
        ty: TyId,
    },
}

impl fmt::Display for UnifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mismatch { expected, found } => {
                write!(f, "cannot unify `{expected:?}` with `{found:?}`")
            }
            Self::Occurs { variable, ty } => {
                write!(f, "inference variable `{variable:?}` occurs in `{ty:?}`")
            }
        }
    }
}

impl std::error::Error for UnifyError {}

#[cfg(test)]
mod tests {
    use super::{InferCtxt, UnifyError};
    use crate::TyCtxt;
    use crate::kind::{IntTy, TyKind, UintTy};

    #[test]
    fn fresh_variables_are_distinct() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let first = infer.fresh_ty_var(&mut ctxt);
        let second = infer.fresh_ty_var(&mut ctxt);
        assert_ne!(first, second);
        assert_eq!(infer.len(), 2);
        assert!(!infer.is_empty());
        assert!(matches!(ctxt.kind(first), TyKind::Infer(_)));
    }

    #[test]
    fn a_variable_unifies_with_a_concrete_type() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let int = ctxt.int_ty(IntTy::I32);
        infer.unify(&ctxt, var, int).expect("variable unifies");
        assert_eq!(infer.resolve_shallow(&ctxt, var), int);
        assert_eq!(infer.resolve_deep(&mut ctxt, var), int);
    }

    #[test]
    fn two_variables_unify_and_share_the_binding() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let first = infer.fresh_ty_var(&mut ctxt);
        let second = infer.fresh_ty_var(&mut ctxt);
        let uint = ctxt.uint_ty(UintTy::U64);

        infer.unify(&ctxt, first, second).expect("variables unify");
        infer.unify(&ctxt, second, uint).expect("variable binds");
        assert_eq!(infer.resolve_shallow(&ctxt, first), uint);
        assert_eq!(infer.resolve_shallow(&ctxt, second), uint);
    }

    #[test]
    fn distinct_scalars_do_not_unify() {
        let ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let uint = ctxt.uint_ty(UintTy::U32);
        let error = infer.unify(&ctxt, int, uint).expect_err("no coercion");
        assert_eq!(
            error,
            UnifyError::Mismatch {
                expected: int,
                found: uint
            }
        );
        assert!(error.to_string().contains("cannot unify"));
    }

    #[test]
    fn integer_widths_do_not_unify() {
        let ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let i8_ty = ctxt.int_ty(IntTy::I8);
        let i64_ty = ctxt.int_ty(IntTy::I64);
        assert!(infer.unify(&ctxt, i8_ty, i64_ty).is_err());
    }

    #[test]
    fn references_unify_structurally() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let pointee = ctxt.int_ty(IntTy::I32);
        let pointer = ctxt.ref_ty(var);
        let expected = ctxt.ref_ty(pointee);

        infer
            .unify(&ctxt, pointer, expected)
            .expect("pointees unify");
        assert_eq!(infer.resolve_shallow(&ctxt, var), pointee);
    }

    #[test]
    fn reference_to_different_scalars_does_not_unify() {
        let ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let i32_ty = ctxt.int_ty(IntTy::I32);
        let i64_ty = ctxt.int_ty(IntTy::I64);
        let left = ctxt.ref_ty(i32_ty);
        let right = ctxt.ref_ty(i64_ty);
        let error = infer
            .unify(&ctxt, left, right)
            .expect_err("pointees differ");
        assert!(matches!(error, UnifyError::Mismatch { .. }));
    }

    #[test]
    fn reference_kinds_do_not_unify() {
        let ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let shared = ctxt.ref_ty(int);
        let exclusive = ctxt.mut_ref_ty(int);
        assert!(matches!(
            infer.unify(&ctxt, shared, exclusive),
            Err(UnifyError::Mismatch { .. })
        ));
    }

    #[test]
    fn a_variable_cannot_contain_itself() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let pointer = ctxt.ref_ty(var);
        let error = infer
            .unify(&ctxt, var, pointer)
            .expect_err("occurs check must reject");
        match error {
            UnifyError::Occurs { variable, ty } => {
                assert_eq!(variable.index(), 0);
                assert_eq!(ty, pointer);
            }
            other => panic!("expected an occurs-check error, got {other:?}"),
        }
        assert_eq!(infer.resolve_shallow(&ctxt, var), var);
    }

    #[test]
    fn the_occurs_check_looks_through_nested_references() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let nested = ctxt.ref_ty(var);
        let inner = ctxt.ref_ty(nested);
        assert!(matches!(
            infer.unify(&ctxt, var, inner),
            Err(UnifyError::Occurs { .. })
        ));
    }

    #[test]
    fn the_error_type_absorbs_any_other() {
        let ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let error_ty = ctxt.error_ty();
        let int = ctxt.int_ty(IntTy::I32);
        infer.unify(&ctxt, error_ty, int).expect("recovery absorbs");
        infer
            .unify(&ctxt, int, error_ty)
            .expect("recovery is symmetric");
        assert_eq!(infer.resolve_shallow(&ctxt, int), int);
    }

    #[test]
    fn deep_resolution_rebuilds_pointer_types() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let pointer = ctxt.ref_ty(var);
        let pointee = ctxt.char_ty();
        infer.unify(&ctxt, var, pointee).expect("variable binds");

        let resolved = infer.resolve_deep(&mut ctxt, pointer);
        let expected = ctxt.ref_ty(pointee);
        assert_eq!(resolved, expected);
        assert_eq!(infer.resolve_deep(&mut ctxt, expected), expected);
    }

    #[test]
    fn deep_resolution_leaves_unbound_variables_alone() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let pointer = ctxt.ref_ty(var);
        assert_eq!(infer.resolve_deep(&mut ctxt, pointer), pointer);
    }

    #[test]
    fn resolution_compresses_chains() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let first = infer.fresh_ty_var(&mut ctxt);
        let second = infer.fresh_ty_var(&mut ctxt);
        let int = ctxt.int_ty(IntTy::I32);

        infer.unify(&ctxt, first, second).expect("variables unify");
        infer.unify(&ctxt, second, int).expect("second binds");
        assert_eq!(infer.resolve_shallow(&ctxt, first), int);
        assert_eq!(
            infer.vars[crate::InferTyId::from_u32(0).unwrap()],
            Some(int)
        );
    }

    #[test]
    fn unifying_a_type_with_itself_is_a_no_op() {
        let ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let unit = ctxt.unit_ty();
        infer.unify(&ctxt, int, int).expect("identical types unify");
        infer.unify(&ctxt, unit, unit).expect("unit unifies");
    }

    #[test]
    fn arrays_unify_element_wise_and_keep_their_length() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let int = ctxt.int_ty(IntTy::I32);
        infer
            .unify(&ctxt, ctxt.array_ty(var, 3), ctxt.array_ty(int, 3))
            .expect("elements unify");
        assert_eq!(infer.resolve_shallow(&ctxt, var), int);

        let error = infer
            .unify(&ctxt, ctxt.array_ty(int, 2), ctxt.array_ty(int, 3))
            .expect_err("lengths are part of the type");
        assert!(matches!(error, UnifyError::Mismatch { .. }));
    }

    #[test]
    fn the_occurs_check_looks_through_arrays() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let array = ctxt.array_ty(var, 1);
        assert!(matches!(
            infer.unify(&ctxt, var, array),
            Err(UnifyError::Occurs { .. })
        ));
    }

    #[test]
    fn deep_resolution_rebuilds_array_types() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        let array = ctxt.array_ty(var, 2);
        let int = ctxt.int_ty(IntTy::I32);
        infer.unify(&ctxt, var, int).expect("variable binds");
        assert_eq!(infer.resolve_deep(&mut ctxt, array), ctxt.array_ty(int, 2));
    }
}
