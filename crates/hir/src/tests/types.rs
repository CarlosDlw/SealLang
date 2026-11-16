//! Type-lowering tests.

use crate::StmtKind;
use seal_ty::{FloatTy, IntTy, TyKind, UintTy};

use super::support::{Fixture, function, lower_clean};

#[test]
fn annotations_lower_to_interned_types() {
    let (fixture, hir) = lower_clean("func f(x: *i32) -> f64 { let y: u8 = 1; 0.0 }");
    let (_, body) = function(&fixture, &hir, "f");

    let TyKind::Ref { pointee: inner, .. } = hir.ty_ctxt().kind(body.params[0].ty) else {
        panic!("parameter type must be a reference");
    };
    assert_eq!(hir.ty_ctxt().kind(inner), TyKind::Int(IntTy::I32));
    assert_eq!(
        hir.ty_ctxt().kind(body.ret.expect("declared return type")),
        TyKind::Float(FloatTy::F64)
    );

    let StmtKind::Let { ty, .. } = &hir.stmt(hir.block(body.root).statements[0]).kind else {
        panic!("expected a let");
    };
    assert_eq!(
        hir.ty_ctxt().kind(ty.expect("declared binding type")),
        TyKind::Uint(UintTy::U8)
    );
}

#[test]
fn exclusive_annotations_lower_to_exclusive_references() {
    let (fixture, hir) = lower_clean("func f(p: *mut i32) -> *mut i32 { p }");
    let (_, body) = function(&fixture, &hir, "f");
    let TyKind::Ref { kind, pointee } = hir.ty_ctxt().kind(body.params[0].ty) else {
        panic!("parameter type must be a reference");
    };
    assert!(kind.is_exclusive());
    assert_eq!(hir.ty_ctxt().kind(pointee), TyKind::Int(IntTy::I32));
    assert_eq!(body.ret.expect("return type"), body.params[0].ty);
}

#[test]
fn equal_annotations_share_one_id() {
    let (fixture, hir) = lower_clean("func f(a: i32, b: i32) -> i32 { a }");
    let (_, body) = function(&fixture, &hir, "f");

    assert_eq!(body.params[0].ty, body.params[1].ty);
    assert_eq!(body.ret.expect("return type"), body.params[0].ty);
}

#[test]
fn missing_annotations_stay_absent() {
    let (fixture, hir) = lower_clean("func f() { let x = 1; }");
    let (_, body) = function(&fixture, &hir, "f");

    assert!(body.ret.is_none());
    let StmtKind::Let { ty, init, .. } = &hir.stmt(hir.block(body.root).statements[0]).kind else {
        panic!("expected a let");
    };
    assert!(ty.is_none());
    assert!(init.is_some());
}

#[test]
fn a_recovered_type_annotation_lowers_to_the_error_type() {
    let fixture = Fixture::new("func f() -> Nope { }");
    let hir = fixture.lower();

    assert!(fixture.codes().iter().any(|code| code.as_str() == "E2002"));
    let (_, body) = function(&fixture, &hir, "f");
    assert_eq!(
        hir.ty_ctxt().kind(body.ret.expect("recorded return type")),
        TyKind::Error
    );
}
