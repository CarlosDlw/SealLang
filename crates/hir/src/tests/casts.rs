//! Cast lowering: resolved targets and operand reachability.

use seal_ty::{FloatTy, IntTy, RefKind, TyKind, UintTy};

use super::support::{Fixture, assert_invariants, function, lower_clean, tail};

fn tail_cast(hir: &crate::Hir, body: &crate::Body) -> (crate::HirExprId, seal_ty::TyId) {
    let expr = tail(hir, body);
    let crate::ExprKind::Cast { operand, ty, .. } = hir.expr(expr).kind else {
        panic!("expected a cast, got {:?}", hir.expr(expr).kind);
    };
    (operand, ty)
}

#[test]
fn a_primitive_target_resolves_to_its_integer_type() {
    let (fixture, hir) = lower_clean("func f(value: i32) -> u8 { value as u8 }");
    let (_, body) = function(&fixture, &hir, "f");
    let (_, ty) = tail_cast(&hir, body);
    assert_eq!(hir.ty_ctxt().kind(ty), TyKind::Uint(UintTy::U8));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_signed_and_a_float_target_resolve() {
    let (fixture, hir) = lower_clean(
        "func f(value: i64, other: f32) -> i32 { value as i32 } func g(value: f32) -> f64 { value as f64 }",
    );
    let (_, body) = function(&fixture, &hir, "f");
    let (_, ty) = tail_cast(&hir, body);
    assert_eq!(hir.ty_ctxt().kind(ty), TyKind::Int(IntTy::I32));
    let (_, body) = function(&fixture, &hir, "g");
    let (_, ty) = tail_cast(&hir, body);
    assert_eq!(hir.ty_ctxt().kind(ty), TyKind::Float(FloatTy::F64));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_pointer_target_keeps_its_pointee_and_mutability() {
    let (fixture, hir) = lower_clean("func f(value: *i32) -> *mut u8 { value as *mut u8 }");
    let (_, body) = function(&fixture, &hir, "f");
    let (_, ty) = tail_cast(&hir, body);
    let TyKind::Ref { pointee, kind } = hir.ty_ctxt().kind(ty) else {
        panic!(
            "expected a pointer target, got {:?}",
            hir.ty_ctxt().kind(ty)
        );
    };
    assert_eq!(kind, RefKind::Exclusive);
    assert_eq!(hir.ty_ctxt().kind(pointee), TyKind::Uint(UintTy::U8));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_named_target_resolves_to_its_adt() {
    let (fixture, hir) =
        lower_clean("struct Point { x: i32 } func f(value: *Point) -> Point { *value as Point }");
    let (_, body) = function(&fixture, &hir, "f");
    let (_, ty) = tail_cast(&hir, body);
    assert!(matches!(hir.ty_ctxt().kind(ty), TyKind::Adt { .. }));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_trait_object_target_resolves() {
    let (fixture, hir) =
        lower_clean("trait Shape {} func f(value: *i32) -> *dyn Shape { value as *dyn Shape }");
    let (_, body) = function(&fixture, &hir, "f");
    let (_, ty) = tail_cast(&hir, body);
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(ty) else {
        panic!("expected a pointer to a trait object");
    };
    assert!(matches!(hir.ty_ctxt().kind(pointee), TyKind::Dyn { .. }));
    // The cast target and the return annotation each record one use.
    assert_eq!(hir.dyn_types().len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn the_operand_stays_a_child_of_the_cast() {
    let (fixture, hir) = lower_clean("func f(value: i32) -> i64 { value as i64 }");
    let (_, body) = function(&fixture, &hir, "f");
    let (operand, _) = tail_cast(&hir, body);
    assert!(matches!(
        hir.expr(operand).kind,
        crate::ExprKind::Path {
            res: crate::Res::Local(_),
            ..
        }
    ));
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_unknown_target_reports_once_and_stays_reachable() {
    let fixture = Fixture::new("func f(value: i32) -> i32 { value as Missing }");
    let hir = fixture.lower();
    fixture.expect("E2002", "cannot find type");
    let (_, body) = function(&fixture, &hir, "f");
    let (operand, ty) = tail_cast(&hir, body);
    assert_eq!(hir.ty_ctxt().kind(ty), TyKind::Error);
    assert!(matches!(
        hir.expr(operand).kind,
        crate::ExprKind::Path {
            res: crate::Res::Local(_),
            ..
        }
    ));
    assert_eq!(
        fixture
            .codes()
            .iter()
            .filter(|code| code.as_str() == "E2002")
            .count(),
        1
    );
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_unknown_operand_reports_once_and_keeps_the_target() {
    let fixture = Fixture::new("func f() -> i64 { missing as i64 }");
    let hir = fixture.lower();
    fixture.expect("E2002", "cannot find `missing`");
    let (_, body) = function(&fixture, &hir, "f");
    let (_, ty) = tail_cast(&hir, body);
    assert_eq!(hir.ty_ctxt().kind(ty), TyKind::Int(IntTy::I64));
    assert_eq!(
        fixture
            .codes()
            .iter()
            .filter(|code| code.as_str() == "E2002")
            .count(),
        1
    );
    assert_invariants(&fixture, &hir);
}
