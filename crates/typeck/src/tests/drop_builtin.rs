//! Type checking of the built-in `drop(value)`.

use seal_hir::StmtKind;
use seal_ty::TyKind;

use super::support::{check_clean, check_errors, expr_ty, function, statements};

#[test]
fn drop_returns_unit() {
    let source = "\
struct Point { x: i32 } \
func release(point: Point) { let leftover = drop(point); }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "release");
    let statement = statements(&hir, body)[0];
    let init = match hir.stmt(statement).kind.clone() {
        StmtKind::Let {
            init: Some(init), ..
        } => init,
        other => panic!("expected a let with an initializer, got {other:?}"),
    };
    assert!(matches!(
        hir.ty_ctxt().kind(expr_ty(&results, init)),
        TyKind::Unit
    ));
}

#[test]
fn dropping_a_literal_is_allowed() {
    check_clean("func main() { drop(1); }");
}

#[test]
fn dropping_a_shared_reference_is_allowed() {
    check_clean("func main() { let value = 5; drop(&value); }");
}

#[test]
fn dropping_a_pointer_is_allowed() {
    check_clean("func main() { let value = 5; let pointer: *i32 = &value; drop(pointer); }");
}

#[test]
fn dropping_a_copy_pointee_is_allowed() {
    check_clean("func main() { let value = 5; let pointer: *i32 = &value; drop(*pointer); }");
}

#[test]
fn dropping_a_static_is_rejected() {
    let source = "\
struct Token { value: i32 } \
static VALUE: Token = Token { value: 1 }; \
func main() { drop(VALUE); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3058", "out of a static");
}

#[test]
fn dropping_in_a_return_expression_does_not_match_the_return_type() {
    let source = "\
struct Point { x: i32 } \
func main() -> i32 { let point = Point { x: 1 }; return drop(point); }";
    let (fixture, _, _) = check_errors(source);
    assert!(
        !fixture.codes().is_empty(),
        "`drop` returns `()` and cannot stand for an `i32` result"
    );
}

#[test]
fn dropping_a_non_copy_pointee_is_rejected() {
    let source = "\
struct Point { x: i32 } \
func main() { \
    let mut value = Point { x: 1 }; \
    let pointer: *mut Point = &mut value; \
    drop(*pointer); \
}";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}
