//! Intrinsic method lookup, signatures, receiver modes, and diagnostics.

use seal_hir::ExprKind;

use crate::{MethodKind, ReceiverAdjust, ReceiverForm};

use super::support::{check_clean, check_errors, expect_ty, expr_ty, function, tail, ty_text};

#[test]
fn scalar_methods_keep_their_type_and_chain() {
    let (fixture, hir, results) = check_clean("func main() -> i32 { (-5).abs().min(10) }");
    let (_, body) = function(&fixture, &hir, "main");
    let outer = tail(&hir, body);
    expect_ty(&hir, expr_ty(&results, outer), "i32");
    let resolved = results.method(outer).expect("the outer call is resolved");
    assert_eq!(resolved.arg_tys.len(), 1);
    let ExprKind::MethodCall { receiver, .. } = hir.expr(outer).kind else {
        panic!("expected a method call");
    };
    assert!(results.method(receiver).is_some());
}

#[test]
fn a_slice_receiver_is_a_fat_pointer_value() {
    let (fixture, hir, results) =
        check_clean("func length(values: *[i32]) -> usize { values.len() }");
    let (_, body) = function(&fixture, &hir, "length");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("slice length is resolved");
    assert_eq!(resolved.receiver_form, ReceiverForm::Value);
    expect_ty(&hir, resolved.ret_ty, "usize");
}

#[test]
fn an_exclusive_slice_accepts_modifying_methods_without_a_mutable_binding() {
    check_clean(
        "func reverse(values: *mut [i32]) { values.reverse(); } func fill(values: *mut [i32], value: i32) { values.fill(value); }",
    );
}

#[test]
fn a_mutable_array_reference_allows_array_methods() {
    check_clean("func reverse(values: *mut [i32; 3]) { values.reverse(); }");
}

#[test]
fn an_array_temporary_can_be_measured() {
    check_clean("func f() -> usize { [1, 2, 3].len() }");
    check_clean("func f() -> bool { [3, 1, 2].len() == 3usize }");
}

#[test]
fn unknown_methods_are_reported() {
    let (fixture, _, _) = check_errors("func f() -> i32 { 1.unknown() }");
    fixture.expect("E3027", "no method named `unknown`");
}

#[test]
fn method_arity_is_checked() {
    let (fixture, _, _) = check_errors("func f() -> i32 { 1.min() }");
    fixture.expect("E3028", "takes 1 argument");

    let (fixture, _, _) = check_errors("func f() -> i32 { 1.abs(2) }");
    fixture.expect("E3028", "takes 0 argument");
}

#[test]
fn modifying_methods_require_a_writable_receiver() {
    let (fixture, _, _) = check_errors("func f() { let a = [1, 2]; a.reverse(); }");
    fixture.expect("E3029", "not a mutable place");

    let (fixture, _, _) = check_errors("func f(v: *[i32]) { v.reverse(); }");
    fixture.expect("E3029", "shared slice");

    let (fixture, _, _) = check_errors("func f() { let flag = false; flag.toggle(); }");
    fixture.expect("E3029", "not a mutable place");
}

#[test]
fn fill_requires_a_copy_element() {
    let (fixture, _, _) = check_errors(
        "struct Point { x: i32 } func f() { let mut points = [Point { x: 1 }]; points.fill(Point { x: 2 }); }",
    );
    fixture.expect("E3030", "not `Copy`");
}

#[test]
fn fill_accepts_copy_elements_on_arrays_and_slices() {
    check_clean("func f() { let mut values = [1, 2]; values.fill(9); }");
    check_clean("func f(values: *mut [i32], value: i32) { values.fill(value); }");
}

#[test]
fn a_generic_mutable_receiver_reborrows_shared_for_a_read_only_method() {
    let (fixture, hir, results) = check_clean(
        "struct Box<T> { value: T } \
         impl<T> Box<T> { \
           func read(self: *Box<T>) -> T { (*self).value } \
           func read_through_mut(self: *mut Box<T>) -> T { self.read() } \
         } \
         func main() -> i32 { let mut boxed = Box { value: 1 }; boxed.read_through_mut() }",
    );
    let (_, body) = function(&fixture, &hir, "read_through_mut");
    let resolved = results.method(tail(&hir, body)).expect("read is resolved");
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::SharedReborrow);
    assert!(matches!(resolved.kind, MethodKind::User { .. }));
}

#[test]
fn method_result_types_are_the_documented_ones() {
    let (fixture, hir, results) = check_clean(
        "func f(c: char) -> u32 { c.to_u32() } func g(b: bool) -> i32 { b.to_i32() } func h() -> u32 { 7.count_ones() }",
    );
    for (name, expected) in [("f", "u32"), ("g", "i32"), ("h", "u32")] {
        let (_, body) = function(&fixture, &hir, name);
        assert_eq!(ty_text(&hir, expr_ty(&results, tail(&hir, body))), expected);
    }
}
