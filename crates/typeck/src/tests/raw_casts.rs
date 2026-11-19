//! Casts between references, raw pointers, and pointer-sized integers.

use super::support::{check_clean, check_errors, expr_ty, function, tail, ty_text_with};
use seal_hir::HirExprId;
use seal_ty::CastKind;

/// Checks a clean function named `f` whose tail is the expected cast.
fn expect_cast(source: &str, expected: CastKind, target: &str) {
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "f");
    let expr = cast_expr(&hir, body);
    assert_eq!(results.cast_kind(expr), Some(expected));
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, expr)),
        target
    );
}

fn cast_expr(hir: &seal_hir::Hir, body: &seal_hir::Body) -> HirExprId {
    let expr = tail(hir, body);
    assert!(
        matches!(hir.expr(expr).kind, seal_hir::ExprKind::Cast { .. }),
        "expected a cast, got {:?}",
        hir.expr(expr).kind
    );
    expr
}

fn expect_error(source: &str, code: &str) {
    let (fixture, _, _) = check_errors(source);
    fixture.expect(code, "cannot cast");
}

#[test]
fn a_shared_reference_becomes_a_read_only_raw_pointer() {
    expect_cast(
        "func f(value: i32) -> *raw i32 { &value as *raw i32 }",
        CastKind::RefToRaw { mutable: false },
        "*raw i32",
    );
}

#[test]
fn a_mutable_reference_can_become_either_raw_pointer_kind() {
    expect_cast(
        "func f(value: *mut i32) -> *raw mut i32 { value as *raw mut i32 }",
        CastKind::RefToRaw { mutable: true },
        "*raw mut i32",
    );
    expect_cast(
        "func f(value: *mut i32) -> *raw i32 { value as *raw i32 }",
        CastKind::RefToRaw { mutable: false },
        "*raw i32",
    );
}

#[test]
fn a_shared_reference_cannot_gain_write_permission() {
    expect_error(
        "func f(value: *i32) -> *raw mut i32 { value as *raw mut i32 }",
        "E3074",
    );
}

#[test]
fn raw_pointers_change_pointee_and_mutability() {
    expect_cast(
        "func f(pointer: *raw i32) -> *raw mut u8 { pointer as *raw mut u8 }",
        CastKind::RawToRaw,
        "*raw mut u8",
    );
    expect_cast(
        "func f(pointer: *raw mut i32) -> *raw i32 { pointer as *raw i32 }",
        CastKind::RawToRaw,
        "*raw i32",
    );
}

#[test]
fn raw_pointers_convert_to_and_from_pointer_sized_integers() {
    expect_cast(
        "func f(pointer: *raw mut i32) -> usize { pointer as usize }",
        CastKind::RawToInt,
        "usize",
    );
    expect_cast(
        "func f(pointer: *raw i32) -> isize { pointer as isize }",
        CastKind::RawToInt,
        "isize",
    );
    expect_cast(
        "func f() -> *raw i32 { 0usize as *raw i32 }",
        CastKind::IntToRaw,
        "*raw i32",
    );
}

#[test]
fn only_pointer_sized_integers_convert() {
    expect_error(
        "func f(pointer: *raw i32) -> u64 { pointer as u64 }",
        "E3074",
    );
    expect_error(
        "func f(pointer: *raw i32) -> f64 { pointer as f64 }",
        "E3074",
    );
    expect_error("func f() -> *raw i32 { 0u32 as *raw i32 }", "E3074");
}

#[test]
fn fat_and_thin_pointers_do_not_convert_directly() {
    expect_error(
        "func f(pointer: *raw i32) -> *raw [u8] { pointer as *raw [u8] }",
        "E3074",
    );
    expect_error(
        "func f(pointer: *raw [u8]) -> *raw u8 { pointer as *raw u8 }",
        "E3074",
    );
    expect_error("func f() -> *raw [i32] { 0usize as *raw [i32] }", "E3074");
}

#[test]
fn fat_pointers_with_compatible_metadata_convert() {
    expect_cast(
        "func f(pointer: *raw [i32]) -> *raw [u8] { pointer as *raw [u8] }",
        CastKind::RawToRaw,
        "*raw [u8]",
    );
    expect_cast(
        "func f(value: *[i32]) -> *raw [i32] { value as *raw [i32] }",
        CastKind::RefToRaw { mutable: false },
        "*raw [i32]",
    );
    expect_cast(
        "func f(value: *str) -> *raw str { value as *raw str }",
        CastKind::RefToRaw { mutable: false },
        "*raw str",
    );
}

#[test]
fn a_reference_can_unsize_into_a_raw_trait_object() {
    expect_cast(
        "trait Shape { func area(self: *Self) -> i32; } \
         struct Square { side: i32 } \
         impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } } \
         func f(value: *Square) -> *raw dyn Shape { value as *raw dyn Shape }",
        CastKind::RefToRaw { mutable: false },
        "*raw dyn Shape",
    );
    expect_cast(
        "trait Shape { func area(self: *Self) -> i32; } \
         struct Square { side: i32 } \
         impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } } \
         func f(value: *mut Square) -> *raw mut dyn Shape { value as *raw mut dyn Shape }",
        CastKind::RefToRaw { mutable: true },
        "*raw mut dyn Shape",
    );
}

#[test]
fn unsizing_to_a_trait_object_checks_permission_and_implementation() {
    expect_error(
        "trait Shape { func area(self: *Self) -> i32; } \
         struct Square { side: i32 } \
         impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } } \
         func f(value: *Square) -> *raw mut dyn Shape { value as *raw mut dyn Shape }",
        "E3074",
    );
    let (fixture, _, _) = check_errors(
        "trait Shape { func area(self: *Self) -> i32; } \
         struct Other { value: i32 } \
         func f(value: *Other) -> *raw dyn Shape { value as *raw dyn Shape }",
    );
    fixture.expect("E3046", "does not implement");
}

#[test]
fn a_raw_pointer_does_not_unsize_into_a_trait_object() {
    expect_error(
        "trait Shape { func area(self: *Self) -> i32; } \
         func f(pointer: *raw i32) -> *raw dyn Shape { pointer as *raw dyn Shape }",
        "E3074",
    );
}

#[test]
fn a_reference_cast_keeps_the_pointee() {
    expect_error(
        "func f(value: *i32) -> *raw i64 { value as *raw i64 }",
        "E3074",
    );
}
