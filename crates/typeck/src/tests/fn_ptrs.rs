//! First-class Seal function pointers.

use seal_hir::{ExprKind, Hir, HirExprId, Res};
use seal_ty::{TyId, TyKind};

use crate::TypeckResults;

use super::support::{check_clean, check_errors, expect_ty, expr_ty, function, tail};

/// Returns one function item used as a value and its type.
fn function_value(hir: &Hir, results: &TypeckResults) -> (HirExprId, TyId) {
    for (id, expr) in hir.expr_arena().iter_enumerated() {
        if matches!(
            &expr.kind,
            ExprKind::Path {
                res: Res::Function(_),
                ..
            }
        ) && results.function_address(id).is_some()
        {
            return (id, expr_ty(results, id));
        }
    }
    panic!("the program has no function item used as a value");
}

#[test]
fn a_function_pointer_binding_calls_through_its_signature() {
    let (fixture, hir, results) = check_clean(
        "func add(a: i32, b: i32) -> i32 { a + b } \
         func main() -> i32 { let f: func(i32, i32) -> i32 = add; f(2, 3) }",
    );
    let (_, main) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, main)), "i32");

    let (_, ty) = function_value(&hir, &results);
    expect_ty(&hir, ty, "func(i32, i32) -> i32");
    assert!(matches!(hir.ty_ctxt().kind(ty), TyKind::FnPtr(_)));
}

#[test]
fn function_pointer_types_are_structural_across_positions() {
    let source = "\
        func identity(value: i32) -> i32 { value } \
        func take(callback: func(i32) -> i32) -> i32 { callback(1) } \
        func make() -> func(i32) -> i32 { identity } \
        func main() -> i32 { take(identity) }";
    let (fixture, hir, results) = check_clean(source);

    let mut types = Vec::new();
    for (id, expr) in hir.expr_arena().iter_enumerated() {
        if matches!(
            &expr.kind,
            ExprKind::Path {
                res: Res::Function(_),
                ..
            }
        ) && results.function_address(id).is_some()
        {
            types.push(expr_ty(&results, id));
        }
    }
    assert!(
        types.len() >= 2,
        "the argument and the returned value use the same type"
    );
    for ty in &types {
        expect_ty(&hir, *ty, "func(i32) -> i32");
        assert_eq!(*ty, types[0], "every occurrence is the same interned type");
    }

    let (_, main) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, main)), "i32");
}

#[test]
fn function_pointers_flow_through_fields_arrays_and_tuples() {
    let source = "\
        struct Handlers { run: func(i32) -> i32 } \
        func identity(value: i32) -> i32 { value } \
        func pick(callbacks: [func(i32) -> i32; 1], \
                  pair: (func(i32) -> i32, i32), \
                  handlers: Handlers) -> func(i32) -> i32 { callbacks[0] } \
        func run(handlers: Handlers, value: i32) -> i32 { (handlers.run)(value) } \
        func main() -> i32 { let pair = (identity, 1); pair.1 }";
    check_clean(source);
}

#[test]
fn a_function_pointer_call_checks_arity_and_argument_types() {
    let source = "\
        func identity(value: i32) -> i32 { value } \
        func main() { let f = identity; f(); f(true); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3008", "takes 1 argument(s) but 0 were supplied");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_function_pointer_is_copy() {
    let (fixture, hir, results) = check_clean(
        "func identity(value: i32) -> i32 { value } \
         func main() -> i32 { let f = identity; let g = f; g(1) }",
    );
    let (_, ty) = function_value(&hir, &results);
    assert!(hir.ty_ctxt().is_copy(ty));
    let _ = fixture;
}

#[test]
fn an_unsafe_function_pointer_requires_unsafe_to_call() {
    let source = "\
        unsafe func bump(value: i32) -> i32 { value } \
        func main() -> i32 { let f: unsafe func(i32) -> i32 = bump; f(1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3070", "requires an `unsafe` block");
}

#[test]
fn calling_an_unsafe_pointer_inside_unsafe_is_allowed() {
    let source = "\
        unsafe func bump(value: i32) -> i32 { value } \
        func main() -> i32 { let f: unsafe func(i32) -> i32 = bump; unsafe { f(1) } }";
    let (fixture, _, _) = check_clean(source);
    let _ = fixture;
}

#[test]
fn a_safe_function_becomes_an_unsafe_pointer() {
    let source = "\
        func identity(value: i32) -> i32 { value } \
        func main() -> i32 { let f: unsafe func(i32) -> i32 = identity; unsafe { f(1) } }";
    let (fixture, _, _) = check_clean(source);
    let _ = fixture;
}

#[test]
fn an_unsafe_pointer_cannot_become_safe() {
    let source = "\
        unsafe func bump(value: i32) -> i32 { value } \
        func main() -> i32 { let f: func(i32) -> i32 = bump; f(1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3098", "cannot drop `unsafe`");
}

#[test]
fn a_different_signature_is_rejected() {
    let source = "\
        func identity(value: i32) -> i32 { value } \
        func main() { let f: func(bool) -> i32 = identity; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3098", "signatures differ");
}

#[test]
fn a_c_function_is_reached_only_through_an_explicit_cast() {
    let source = "\
        extern \"c\" { func foreign(value: i32) -> i32; } \
        func main() { let f = foreign; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3099", "external function");
}

#[test]
fn a_method_with_a_receiver_is_not_a_function_pointer() {
    // Resolution already rejects an associated path to a method, before type
    // checking could build a pointer type for it. The type checker keeps the
    // `E3099` branch as a defensive check for any future path that reaches it.
    let source = "\
        struct Token { value: i32 } \
        impl Token { func bump(self: *mut Token) { } } \
        func main() { let f = Token::bump; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2019", "is a method, not an associated function");
}

#[test]
fn a_trait_function_is_not_a_function_pointer() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Factory { func make() -> i32 { 1 } } \
        impl Factory for Circle { } \
        func main() { let f = Circle::make; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3099", "trait method");
}

#[test]
fn an_explicit_cast_builds_a_function_pointer() {
    let source = "\
        func identity(value: i32) -> i32 { value } \
        func main() -> i32 { \
            let f = identity as func(i32) -> i32; \
            let g = identity as unsafe func(i32) -> i32; \
            unsafe { g(f(1)) } \
        }";
    let (fixture, _, _) = check_clean(source);
    let _ = fixture;
}

#[test]
fn an_explicit_cast_keeps_the_safety_rule() {
    let source = "\
        unsafe func bump(value: i32) -> i32 { value } \
        func main() { let f = bump as func(i32) -> i32; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3098", "cannot drop `unsafe`");
}

#[test]
fn an_explicit_cast_keeps_the_signature() {
    let source = "\
        func identity(value: i32) -> i32 { value } \
        func main() { let f = identity as func(bool) -> i32; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3098", "signatures differ");
}

#[test]
fn a_function_pointer_result_must_be_sized() {
    let source = "func use(callback: func(str)) {} func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3010", "unsized");
}

#[test]
fn a_seal_function_pointer_is_never_variadic() {
    let source = "func use(callback: func(i32, ...)) {} func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3099", "cannot be variadic");
}

#[test]
fn a_c_signature_is_not_a_value_type() {
    let source = "func use(callback: extern \"c\" func(i32)) {} func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3089", "must appear behind `*raw`");
}

#[test]
fn function_pointers_have_no_equality() {
    let source = "\
        func identity(value: i32) -> i32 { value } \
        func main() -> bool { let f = identity; let g = identity; f == g }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3002", "cannot be applied");
}
