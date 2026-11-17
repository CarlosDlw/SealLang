//! Slice references, ranges, and the built-in `len` lowering.

use crate::{ExprKind, Res};

use super::support::{function, lower_clean, tail};

#[test]
fn a_range_borrow_lowers_to_a_slice_reference() {
    let (fixture, hir) = lower_clean("func f(a: [i32; 4]) -> *[i32] { &a[1..3] }");
    let (_, body) = function(&fixture, &hir, "f");
    let ExprKind::SliceRef {
        start, end, kind, ..
    } = hir.expr(tail(&hir, body)).kind
    else {
        panic!("expected a slice reference");
    };
    assert!(start.is_some());
    assert!(end.is_some());
    assert_eq!(kind, seal_ty::RefKind::Shared);
}

#[test]
fn a_mutable_range_keeps_the_exclusive_kind() {
    let (fixture, hir) =
        lower_clean("func f() -> *mut [i32] { let mut values = [1, 2]; &mut values[..] }");
    let (_, body) = function(&fixture, &hir, "f");
    let ExprKind::SliceRef { kind, .. } = hir.expr(tail(&hir, body)).kind else {
        panic!("expected a slice reference");
    };
    assert_eq!(kind, seal_ty::RefKind::Exclusive);
}

#[test]
fn len_lowers_to_its_own_expression() {
    let (fixture, hir) = lower_clean("func f(a: [i32; 4]) -> usize { len(&a[..]) }");
    let (_, body) = function(&fixture, &hir, "f");
    let ExprKind::Len { operand } = hir.expr(tail(&hir, body)).kind else {
        panic!("expected the len built-in");
    };
    assert!(matches!(hir.expr(operand).kind, ExprKind::SliceRef { .. }));
}

#[test]
fn a_user_function_named_len_shadows_the_builtin() {
    let (fixture, hir) = lower_clean(
        "func len(value: *[i32]) -> usize { 0 }\n\
         func f(a: [i32; 4]) -> usize { len(&a[..]) }",
    );
    let (_, body) = function(&fixture, &hir, "f");
    let ExprKind::Call { callee, args } = &hir.expr(tail(&hir, body)).kind else {
        panic!("expected a normal call");
    };
    assert!(matches!(
        hir.expr(*callee).kind,
        ExprKind::Path {
            res: Res::Function(_),
            ..
        }
    ));
    assert_eq!(args.len(), 1);
}
