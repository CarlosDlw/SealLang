//! Explicit numeric casts: accepted families, recorded kinds, diagnostics.

use seal_hir::{ExprKind, HirExprId};
use seal_ty::{FloatTy, IntTy, NumericCastKind, TyKind, UintTy};

use super::support::{check_clean, check_errors, expr_ty, function, tail, ty_text_with};

fn tail_cast(hir: &seal_hir::Hir, body: &seal_hir::Body) -> HirExprId {
    let expr = tail(hir, body);
    assert!(
        matches!(hir.expr(expr).kind, ExprKind::Cast { .. }),
        "expected a cast, got {:?}",
        hir.expr(expr).kind
    );
    expr
}

fn expect_kind(results: &crate::TypeckResults, expr: HirExprId, kind: NumericCastKind) {
    assert_eq!(
        results.cast_kind(expr),
        Some(kind),
        "unexpected cast kind for {expr:?}"
    );
}

fn expect_one_error(source: &str, code: &str, fragment: &str) {
    let (fixture, _, _) = check_errors(source);
    fixture.expect(code, fragment);
    let count = fixture
        .codes()
        .iter()
        .filter(|found| found.as_str() == code)
        .count();
    assert_eq!(
        count,
        1,
        "expected exactly one {code}, got {:?}",
        fixture.codes()
    );
}

#[test]
fn an_integer_to_integer_cast_records_its_kind_and_target_type() {
    let (fixture, hir, results) = check_clean(
        "func f(value: i32) -> u8 { 300 as u8 } func g(value: i32) -> u8 { value as u8 }",
    );
    for name in ["f", "g"] {
        let (_, body) = function(&fixture, &hir, name);
        let expr = tail_cast(&hir, body);
        let ty = expr_ty(&results, expr);
        assert_eq!(hir.ty_ctxt().kind(ty), TyKind::Uint(UintTy::U8));
        expect_kind(&results, expr, NumericCastKind::IntToInt);
    }
}

#[test]
fn a_literal_operand_keeps_its_default_type() {
    let (fixture, hir, results) = check_clean("func f() -> u8 { 1 as u8 }");
    let (_, body) = function(&fixture, &hir, "f");
    let expr = tail_cast(&hir, body);
    let ExprKind::Cast { operand, .. } = hir.expr(expr).kind else {
        panic!("expected a cast");
    };
    let source = expr_ty(&results, operand);
    assert_eq!(hir.ty_ctxt().kind(source), TyKind::Int(IntTy::I32));
    assert_eq!(ty_text_with(&fixture, &hir, source), "i32");
}

#[test]
fn every_numeric_family_is_accepted_with_its_kind() {
    let (fixture, hir, results) = check_clean(
        "\
         func a(value: i64) -> u16 { value as u16 }
         func b(value: u8) -> f32 { value as f32 }
         func c(value: f64) -> i32 { value as i32 }
         func d(value: f32) -> f64 { value as f64 }
         func e(value: f64) -> f32 { value as f32 }",
    );
    let expected = [
        ("a", NumericCastKind::IntToInt),
        ("b", NumericCastKind::IntToFloat),
        ("c", NumericCastKind::FloatToInt),
        ("d", NumericCastKind::FloatToFloat),
        ("e", NumericCastKind::FloatToFloat),
    ];
    for (name, kind) in expected {
        let (_, body) = function(&fixture, &hir, name);
        let expr = tail_cast(&hir, body);
        expect_kind(&results, expr, kind);
    }
}

#[test]
fn a_same_type_cast_is_a_no_op() {
    let (fixture, hir, results) = check_clean(
        "func a(value: i32) -> i32 { value as i32 } func b(value: f32) -> f32 { value as f32 }",
    );
    let (_, body) = function(&fixture, &hir, "a");
    let expr = tail_cast(&hir, body);
    let ty = expr_ty(&results, expr);
    assert_eq!(hir.ty_ctxt().kind(ty), TyKind::Int(IntTy::I32));
    expect_kind(&results, expr, NumericCastKind::IntToInt);
    let (_, body) = function(&fixture, &hir, "b");
    let expr = tail_cast(&hir, body);
    expect_kind(&results, expr, NumericCastKind::FloatToFloat);
}

#[test]
fn bool_and_char_are_rejected_with_their_named_conversions() {
    expect_one_error(
        "func f(value: bool) -> i32 { value as i32 }",
        "E3048",
        "cannot cast `bool` to `i32`",
    );
    expect_one_error(
        "func f(value: char) -> u32 { value as u32 }",
        "E3048",
        "cannot cast `char` to `u32`",
    );
}

#[test]
fn non_numeric_targets_are_rejected() {
    for (source, message) in [
        (
            "func f(value: i32) -> bool { value as bool }",
            "`i32` to `bool`",
        ),
        (
            "func f(value: i32) -> char { value as char }",
            "`i32` to `char`",
        ),
        (
            "func f(value: i32) -> *i32 { value as *i32 }",
            "`i32` to `*i32`",
        ),
        (
            "func f(value: i32) -> *mut i32 { value as *mut i32 }",
            "`i32` to `*mut i32`",
        ),
        (
            "func f(value: i32) -> str { value as str }",
            "`i32` to `str`",
        ),
        (
            "struct S { x: i32 } func f(value: i32) -> S { value as S }",
            "`i32` to `S`",
        ),
        (
            "func f(value: i32) -> (i32, i32) { value as (i32, i32) }",
            "`i32` to `(i32, i32)`",
        ),
        (
            "func f(value: i32) -> [i32; 2] { value as [i32; 2] }",
            "`i32` to `[i32; 2]`",
        ),
    ] {
        expect_one_error(source, "E3048", message);
    }
}

#[test]
fn a_trait_object_target_is_rejected() {
    expect_one_error(
        "trait Shape {} func f(value: i32) -> *dyn Shape { value as *dyn Shape }",
        "E3048",
        "`i32` to `*dyn Shape`",
    );
}

#[test]
fn a_slice_target_is_rejected() {
    expect_one_error(
        "func f(value: i32) -> *[i32] { value as *[i32] }",
        "E3048",
        "`i32` to `*[i32]`",
    );
}

#[test]
fn the_never_type_is_not_castable() {
    expect_one_error(
        "func f() -> i32 { ({ return 0; }) as i32 }",
        "E3048",
        "cannot cast `!` to `i32`",
    );
}

#[test]
fn a_diverging_expression_flows_into_an_expected_type_without_a_cast() {
    let _ = check_clean("func f() -> i32 { return 0; }");
    let (fixture, hir, results) =
        check_clean("func f() -> i32 { let value: i32 = { return 0; }; value }");
    assert!(fixture.diagnostics().is_empty());
    let (_, body) = function(&fixture, &hir, "f");
    let _ = tail(&hir, body);
    let _ = results;
}

#[test]
fn an_operand_error_does_not_cascade_into_a_cast_diagnostic() {
    let (fixture, _, _) = check_errors("func f() -> i32 { missing as i32 }");
    fixture.expect("E2002", "cannot find `missing`");
    assert_eq!(
        fixture
            .codes()
            .iter()
            .filter(|code| code.as_str() == "E3048")
            .count(),
        0,
        "unexpected cast diagnostic: {:?}",
        fixture.codes()
    );
}

#[test]
fn a_target_error_does_not_cascade_into_a_cast_diagnostic() {
    let (fixture, _, _) = check_errors("func f(value: i32) -> i32 { value as Missing }");
    fixture.expect("E2002", "cannot find type");
    assert_eq!(
        fixture
            .codes()
            .iter()
            .filter(|code| code.as_str() == "E3048")
            .count(),
        0,
        "unexpected cast diagnostic: {:?}",
        fixture.codes()
    );
}

#[test]
fn a_type_parameter_source_is_rejected_with_its_name() {
    expect_one_error(
        "func f<T>(value: T) -> i32 { value as i32 }",
        "E3049",
        "cannot cast type parameter `T` to `i32`",
    );
}

#[test]
fn a_type_parameter_target_is_rejected_with_its_name() {
    expect_one_error(
        "func f<T>(value: i32) -> T { value as T }",
        "E3049",
        "cannot cast `i32` to type parameter `T`",
    );
}

#[test]
fn a_cast_of_a_possibly_uninitialized_local_is_rejected() {
    let (fixture, _, _) = check_errors("func f() -> i32 { let value: i32; value as i64 as i32 }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn a_cast_in_a_local_initializer_records_its_kind() {
    let (fixture, hir, results) =
        check_clean("func f(value: u64) -> f64 { let converted = value as f64; converted }");
    let (_, body) = function(&fixture, &hir, "f");
    let statement = super::support::statements(&hir, body)[0];
    let seal_hir::StmtKind::Let {
        init: Some(expr), ..
    } = hir.stmt(statement).kind
    else {
        panic!("expected an initialized local");
    };
    assert_eq!(results.cast_kind(expr), Some(NumericCastKind::IntToFloat));
    assert_eq!(
        hir.ty_ctxt().kind(expr_ty(&results, expr)),
        TyKind::Float(FloatTy::F64)
    );
}

#[test]
fn a_cast_inside_a_call_argument_is_typed_as_its_target() {
    let (fixture, hir, results) = check_clean(
        "func take(value: i32) -> i32 { value } func f(value: u8) -> i32 { take(value as i32) }",
    );
    let (_, body) = function(&fixture, &hir, "f");
    let call = tail(&hir, body);
    assert!(matches!(hir.expr(call).kind, ExprKind::Call { .. }));
    assert_eq!(
        hir.ty_ctxt().kind(expr_ty(&results, call)),
        TyKind::Int(IntTy::I32)
    );
}
