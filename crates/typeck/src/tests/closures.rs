//! Closure type checking: signature, captures, kind, calls, and coercion.

use seal_def::DefId;
use seal_hir::{CaptureSource, ExprKind, Hir, HirExprId};
use seal_ty::ClosureKind;

use crate::TypeckResults;
use crate::results::{CaptureStorage, Coercion, PlaceRoot, ResolvedCall};

use super::support::{check_clean, check_errors, expect_ty, tail};

/// Returns the first closure expression in the arena and its definition.
fn first_closure(hir: &Hir) -> (HirExprId, DefId) {
    for (id, expr) in hir.expr_arena().iter_enumerated() {
        if let ExprKind::Closure { closure, .. } = expr.kind {
            return (id, closure);
        }
    }
    panic!("the program has no closure");
}

fn closure_kind(results: &TypeckResults, closure: DefId) -> ClosureKind {
    results
        .closure_info(closure)
        .unwrap_or_else(|| panic!("closure {closure:?} has no checked info"))
        .kind
}

fn closure_storages(results: &TypeckResults, closure: DefId) -> Vec<CaptureStorage> {
    results
        .closure_info(closure)
        .unwrap_or_else(|| panic!("closure {closure:?} has no checked info"))
        .captures
        .iter()
        .map(|capture| capture.storage)
        .collect()
}

fn closure_ret(hir: &Hir, results: &TypeckResults, closure: DefId) -> String {
    let signature = results
        .signature(closure)
        .expect("the closure has a signature");
    hir.ty_ctxt().display(signature.ret).to_string()
}

#[test]
fn a_closure_that_only_reads_is_fn() {
    let (_, hir, results) = check_clean("func main() -> i32 { let x = 1; let f = || x + 1; f() }");
    let (_, closure) = first_closure(&hir);
    assert_eq!(closure_kind(&results, closure), ClosureKind::Fn);
}

#[test]
fn writing_a_capture_makes_the_closure_fn_mut() {
    let (_, hir, results) = check_clean(
        "func main() { let mut total = 0; let mut f = || { total = total + 1; }; f(); }",
    );
    let (_, closure) = first_closure(&hir);
    assert_eq!(closure_kind(&results, closure), ClosureKind::FnMut);
    assert_eq!(
        closure_storages(&results, closure),
        vec![CaptureStorage::Exclusive]
    );
}

#[test]
fn moving_a_non_copy_capture_out_makes_the_closure_fn_once() {
    let source = "\
        struct Token { value: i32 } \
        func consume(token: Token) {} \
        func main() { let token = Token { value: 1 }; let take = move || consume(token); }";
    let (_, hir, results) = check_clean(source);
    let (_, closure) = first_closure(&hir);
    assert_eq!(closure_kind(&results, closure), ClosureKind::FnOnce);
}

#[test]
fn a_closure_without_captures_coerces_to_a_function_pointer() {
    let source = "\
        func main() -> i32 { \
            let f: func(i32) -> i32 = |x| x + 1; \
            f(1) \
        }";
    let (_, hir, results) = check_clean(source);
    let (expr, closure) = first_closure(&hir);
    assert_eq!(closure_kind(&results, closure), ClosureKind::Fn);
    let coercion = results.coercion(expr).expect("the closure was coerced");
    assert!(matches!(
        coercion,
        Coercion::ClosureToFnPtr { closure: coerced, .. } if coerced == closure
    ));
}

#[test]
fn move_copies_copy_captures_and_keeps_the_closure_fn() {
    let (_, hir, results) =
        check_clean("func main() -> i32 { let x = 1; let f = move || x + 1; f() }");
    let (_, closure) = first_closure(&hir);
    assert_eq!(closure_kind(&results, closure), ClosureKind::Fn);
    assert_eq!(
        closure_storages(&results, closure),
        vec![CaptureStorage::Value]
    );
}

#[test]
fn without_move_a_non_copy_capture_cannot_be_moved_out() {
    let source = "\
        struct Token { value: i32 } \
        func consume(token: Token) {} \
        func main() { let token = Token { value: 1 }; let take = || consume(token); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3095", "borrowed environment");
    fixture.expect_help("E3095", "add `move`");
}

#[test]
fn without_move_a_copy_capture_is_copied() {
    let (_, hir, results) =
        check_clean("func main() -> i32 { let x = 1; let f = || { let y = x; y }; f() }");
    let (_, closure) = first_closure(&hir);
    assert_eq!(closure_kind(&results, closure), ClosureKind::Fn);
}

#[test]
fn an_unannotated_parameter_without_an_expected_type_is_e3094() {
    let source = "func main() { let f = |x| x; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3094", "cannot infer the type of the closure parameter");
}

#[test]
fn a_mismatched_annotated_signature_reports_both_signatures() {
    let source = "func main() { let f: func(i32) -> i32 = |x: i64| x; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3094", "does not match the expected `func(i32) -> i32`");
}

#[test]
fn the_result_can_be_inferred_from_the_body() {
    let (_, hir, results) = check_clean("func main() -> i32 { let f = || 3; f() }");
    let (_, closure) = first_closure(&hir);
    expect_ty(&hir, results.signature(closure).unwrap().ret, "i32");
}

#[test]
fn the_result_is_inferred_from_an_explicit_return() {
    let source = "\
        func main() -> i32 { \
            let f = || { return 3; }; \
            f() \
        }";
    let (_, hir, results) = check_clean(source);
    let (_, closure) = first_closure(&hir);
    assert_eq!(closure_ret(&hir, &results, closure), "i32");
}

#[test]
fn field_and_index_access_capture_the_base_once() {
    let source = "\
        struct Pair { a: i32, b: i32 } \
        func main() -> i32 { \
            let pair = Pair { a: 1, b: 2 }; \
            let values = [1, 2, 3]; \
            let f = || pair.a + values[1]; \
            f() \
        }";
    let (_, hir, results) = check_clean(source);
    let (_, closure) = first_closure(&hir);
    let definition = hir.closure(closure).expect("the closure has data");
    assert_eq!(definition.captures.len(), 2, "one capture per base");
    assert_eq!(
        closure_storages(&results, closure),
        vec![CaptureStorage::Shared, CaptureStorage::Shared]
    );
}

#[test]
fn a_nested_closure_reads_through_the_outer_environment() {
    let source = "\
        func main() -> i32 { \
            let x = 1; \
            let outer = || { \
                let middle = || { let inner = || x + 1; inner() }; \
                middle() \
            }; \
            outer() \
        }";
    let (_, hir, results) = check_clean(source);
    let mut inner = None;
    for (_, expr) in hir.expr_arena().iter_enumerated() {
        if let ExprKind::Closure { closure, .. } = expr.kind {
            let parent = hir.definitions().get(closure).parent;
            let grandparent = parent.and_then(|parent| hir.definitions().get(parent).parent);
            if grandparent
                .and_then(|grandparent| hir.closure(grandparent))
                .is_some()
            {
                inner = Some(closure);
            }
        }
    }
    let inner = inner.expect("the inner closure exists");
    assert_eq!(
        hir.closure(inner).expect("the closure has data").captures[0].source,
        CaptureSource::Outer(0)
    );
    assert_eq!(closure_kind(&results, inner), ClosureKind::Fn);
}

#[test]
fn a_nested_move_through_a_reference_capture_is_e3095() {
    let source = "\
        struct Token { value: i32 } \
        func consume(token: Token) {} \
        func main() { \
            let token = Token { value: 1 }; \
            let outer = || { let inner = move || consume(token); inner() }; \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3095", "borrowed environment");
}

#[test]
fn mutating_a_non_mut_capture_is_rejected() {
    let source = "func main() { let total = 0; let f = || { total = total + 1; }; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3017", "cannot capture `total` as mutable");
}

#[test]
fn moving_a_non_copy_field_of_a_move_capture_is_fn_once() {
    let source = "\
        struct Token { value: i32 } \
        struct Holder { token: Token } \
        func consume(token: Token) {} \
        func main() { \
            let holder = Holder { token: Token { value: 1 } }; \
            let take = move || consume(holder.token); \
        }";
    let (_, hir, results) = check_clean(source);
    let (_, closure) = first_closure(&hir);
    assert_eq!(closure_kind(&results, closure), ClosureKind::FnOnce);
}

#[test]
fn a_closure_with_captures_cannot_become_a_function_pointer() {
    let source = "func main() { let x = 1; let f: func(i32) -> i32 = move |y: i32| x + y; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3098", "captures 1 value(s)");
}

#[test]
fn calling_an_fn_mut_closure_needs_a_mutable_binding() {
    let source = "\
        func main() { \
            let mut total = 0; \
            let bump = || { total = total + 1; }; \
            bump(); \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3017", "not declared as mutable");
}

#[test]
fn an_explicit_call_method_is_rejected_with_e3096() {
    let source = "func main() -> i32 { let f = |x: i32| x + 1; f.call(1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "not called through the method `call`");
    fixture.expect_help("E3096", "write `f(...)`");
}

#[test]
fn a_concrete_closure_call_records_its_kind() {
    let source = "func main() -> i32 { let f = |x: i32| x; f(1) }";
    let (_, hir, results) = check_clean(source);
    let (id, callable) = hir
        .expr_arena()
        .iter_enumerated()
        .find(|(id, _)| results.call(*id).is_some())
        .map(|(id, _)| (id, results.call(id).unwrap()))
        .expect("the closure call is recorded");
    let _ = id;
    assert!(matches!(
        callable,
        ResolvedCall::Closure {
            kind: ClosureKind::Fn,
            ..
        }
    ));
}

#[test]
fn a_closure_coerces_in_let_return_field_and_tuple_positions() {
    let source = "\
        struct Holder { callback: func(i32) -> i32 } \
        func make() -> func(i32) -> i32 { |x| x + 1 } \
        func main() -> i32 { \
            let holder = Holder { callback: |x| x + 1 }; \
            let pair: (func(i32) -> i32, i32) = (|x| x + 1, 1); \
            (holder.callback)(1) + (make())(2) + (pair.0)(3) \
        }";
    check_clean(source);
}

#[test]
fn a_closure_argument_coerces_at_the_call_boundary() {
    let source = "\
        func apply(callback: func(i32) -> i32) -> i32 { callback(1) } \
        func main() -> i32 { apply(|x| x + 1) }";
    let (_, hir, results) = check_clean(source);
    let (expr, _) = first_closure(&hir);
    assert!(matches!(
        results.coercion(expr),
        Some(Coercion::ClosureToFnPtr { .. })
    ));
}

#[test]
fn a_capture_place_names_the_closure_environment() {
    let source = "func main() -> i32 { let x = 1; let f = || x + 1; f() }";
    let (_, hir, results) = check_clean(source);
    let capture = hir
        .expr_arena()
        .iter_enumerated()
        .find(|(_, expr)| matches!(expr.kind, ExprKind::Capture { .. }))
        .map(|(id, _)| id)
        .expect("the body reads the capture");
    let place = results.place(capture).expect("the capture has a place");
    assert!(matches!(place.root, PlaceRoot::Capture { index: 0, .. }));
}

#[test]
fn an_fn_mut_closure_is_called_through_a_mutable_binding() {
    let source = "\
        func main() -> i32 { \
            let mut total = 0; \
            let mut bump = || { total = total + 1; }; \
            bump(); \
            total \
        }";
    check_clean(source);
}

#[test]
fn a_closure_inside_a_generic_function_uses_the_enclosing_parameters() {
    let source = "\
        func check<T>(value: T) -> i32 { \
            let f = || 1; \
            f() \
        } \
        func main() -> i32 { check(true) }";
    check_clean(source);
}

#[test]
fn a_closure_body_is_typed_although_it_is_not_a_function_body() {
    let source = "\
        func main() -> i32 { \
            let x = 1; \
            let f = || { let y = x + 1; y }; \
            f() \
        }";
    let (_, hir, results) = check_clean(source);
    let (_, closure) = first_closure(&hir);
    let body = hir.body_of(closure).expect("the closure has a body");
    let tail = tail(&hir, body);
    expect_ty(&hir, results.expr_ty(tail).unwrap(), "i32");
}
