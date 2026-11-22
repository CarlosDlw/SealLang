//! Raw pointer dereferences, writes, borrows, and the `unsafe` gate.

use super::support::{check_clean, check_errors, expr_ty, function, tail, ty_text_with};
use seal_ty::{RawPtrKind, TyKind};

#[test]
fn reading_through_a_raw_pointer_requires_unsafe() {
    let (fixture, _, _) = check_errors("func f(pointer: *raw i32) -> i32 { *pointer }");
    fixture.expect("E3070", "requires an `unsafe` block");
}

#[test]
fn reading_inside_unsafe_produces_the_pointee() {
    let (fixture, hir, results) =
        check_clean("func f(pointer: *raw i32) -> i32 { unsafe { *pointer } }");
    let (_, body) = function(&fixture, &hir, "f");
    let expr = tail(&hir, body);
    assert_eq!(ty_text_with(&fixture, &hir, expr_ty(&results, expr)), "i32");
}

#[test]
fn writing_through_a_mutable_raw_pointer_is_allowed_in_unsafe() {
    let (fixture, _, _) =
        check_clean("func f(pointer: *raw mut i32) { unsafe { *pointer = 41; } }");
    assert!(fixture.diagnostics().is_empty());
}

#[test]
fn writing_through_a_read_only_raw_pointer_is_rejected() {
    let (fixture, _, _) = check_errors("func f(pointer: *raw i32) { unsafe { *pointer = 41; } }");
    fixture.expect("E3075", "cannot write through a `*raw T` pointer");
}

#[test]
fn a_pointer_value_can_be_dereferenced_as_an_assignment_target() {
    check_clean(
        "func offset(pointer: *raw mut i32) -> *raw mut i32 { unsafe { pointer.add(1) } } \
         func f(pointer: *raw mut i32) { unsafe { *(offset(pointer)) = 41; } }",
    );
}

#[test]
fn a_pointer_value_can_be_updated_through_a_dereference() {
    check_clean(
        "func offset(pointer: *raw mut i32) -> *raw mut i32 { unsafe { pointer.add(1) } } \
         func f(pointer: *raw mut i32) { unsafe { *(offset(pointer)) += 1; } }",
    );
}

#[test]
fn a_read_only_pointer_value_rejects_a_dereferenced_write() {
    let (fixture, _, _) = check_errors(
        "func offset(pointer: *raw i32) -> *raw i32 { unsafe { pointer.add(1) } } \
         func f(pointer: *raw i32) { unsafe { *(offset(pointer)) = 41; } }",
    );
    fixture.expect("E3075", "cannot write through a `*raw T` pointer");
}

#[test]
fn a_reference_value_can_be_dereferenced_as_an_assignment_target() {
    check_clean(
        "func identity(pointer: *mut i32) -> *mut i32 { pointer } \
         func f(pointer: *mut i32) { *(identity(pointer)) = 41; }",
    );
}

#[test]
fn shared_and_exclusive_borrows_of_a_raw_pointer_are_checked() {
    let (fixture, hir, results) =
        check_clean("func f(pointer: *raw mut i32) -> *mut i32 { unsafe { &mut *pointer } }");
    let (_, body) = function(&fixture, &hir, "f");
    let expr = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expr)),
        "*mut i32"
    );

    let (fixture, hir, results) =
        check_clean("func f(pointer: *raw i32) -> *i32 { unsafe { &*pointer } }");
    let (_, body) = function(&fixture, &hir, "f");
    let expr = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expr)),
        "*i32"
    );
}

#[test]
fn an_exclusive_borrow_of_a_read_only_raw_pointer_is_rejected() {
    let (fixture, _, _) =
        check_errors("func f(pointer: *raw i32) -> *mut i32 { unsafe { &mut *pointer } }");
    fixture.expect("E3075", "cannot borrow through a `*raw T` pointer");
}

#[test]
fn fields_and_slices_are_projectable_through_a_raw_pointer() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32, y: i32 } \
         func f(pointer: *raw Point) -> i32 { unsafe { (*pointer).x } } \
         func g(pointer: *raw [i32]) -> i32 { unsafe { (*pointer)[1] } }",
    );
    for name in ["f", "g"] {
        let (_, body) = function(&fixture, &hir, name);
        let expr = tail(&hir, body);
        assert_eq!(ty_text_with(&fixture, &hir, expr_ty(&results, expr)), "i32");
    }
}

#[test]
fn a_non_copy_value_moves_out_of_a_raw_pointer() {
    let _ = check_clean(
        "struct Payload { value: i32 } \
         func f(pointer: *raw Payload) -> Payload { unsafe { *pointer } }",
    );
}

#[test]
fn a_non_pointer_operand_is_still_rejected_inside_unsafe() {
    let (fixture, _, _) = check_errors("func f() -> i32 { unsafe { *5 } }");
    fixture.expect("E3012", "cannot dereference");
}

#[test]
fn nested_and_empty_unsafe_blocks_are_valid() {
    let _ = check_clean("func f(pointer: *raw i32) -> i32 { unsafe { unsafe { *pointer } } }");
    let _ = check_clean("func f() { unsafe { } }");
}

#[test]
fn a_raw_dereference_records_its_pointer_kind() {
    let (fixture, hir, results) =
        check_clean("func f(pointer: *raw mut i32) -> i32 { unsafe { *pointer } }");
    let (_, body) = function(&fixture, &hir, "f");
    let expr = tail(&hir, body);
    let seal_hir::ExprKind::Unsafe { block } = hir.expr(expr).kind else {
        panic!("expected an unsafe block");
    };
    let deref = hir.block(block).tail.expect("the unsafe block has a value");
    let place = results.place(deref).expect("the dereference is a place");
    assert!(place.contains_raw_deref());
    assert_eq!(place.raw_deref, Some(RawPtrKind::Mut));
    assert_eq!(
        hir.ty_ctxt().kind(expr_ty(&results, expr)),
        TyKind::Int(seal_ty::IntTy::I32)
    );
}
