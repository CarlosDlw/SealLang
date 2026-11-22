//! Type checking of the compiler-backed `mem::forget`.
//!
//! The fixtures compile as the reserved `core` package and call the private
//! `__forget` builtin directly. That is the same node the public wrapper body
//! calls, so these tests isolate the checking rules from name resolution.

use seal_hir::StmtKind;
use seal_ty::TyKind;

use super::support::{check_core_clean, check_errors, expr_ty, function, statements};

#[test]
fn forget_returns_unit() {
    let source = "\
struct Pair { value: i32 } \
func consume(pair: Pair) { let result = __forget(pair); }";
    let (fixture, hir, results) = check_core_clean(source);
    let (_, body) = function(&fixture, &hir, "consume");
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
fn forgetting_a_non_copy_value_is_allowed() {
    check_core_clean("struct Pair { value: i32 }\nfunc consume(pair: Pair) { __forget(pair); }");
}

#[test]
fn forgetting_a_copy_value_is_allowed() {
    check_core_clean("func consume(value: i32) { __forget(value); }");
}

#[test]
fn forgetting_a_generic_value_is_allowed() {
    check_core_clean("func consume<T>(value: T) { __forget(value); }");
}

#[test]
fn forgetting_a_static_is_rejected() {
    let source = "\
struct Pair { value: i32 } \
static VALUE: Pair = Pair { value: 1 }; \
func main() { __forget(VALUE); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3058", "out of a static");
}

#[test]
fn forgetting_a_non_copy_pointee_is_rejected() {
    let source = "\
struct Pair { value: i32 } \
func main() { \
    let mut value = Pair { value: 1 }; \
    let pointer: *mut Pair = &mut value; \
    __forget(*pointer); \
}";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}
