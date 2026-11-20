//! Function-trait bounds: proof, calls, hints, supertraits, and rejections.

use seal_hir::ExprKind;
use seal_ty::ClosureKind;

use crate::results::ResolvedCall;

use super::support::{check_clean, check_errors, expr_statement, function, statements, tail};

const APPLY: &str = "\
    func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) } \
    func twice<F: Fn() -> i32>(f: F) -> i32 { f() + f() } \
    func each<F: FnMut(i32)>(f: F) { let mut f = f; f(1); f(2); } \
    func consume<F: FnOnce() -> i32>(f: F) -> i32 { f() }";

#[test]
fn a_generic_fn_bound_is_called_with_its_declared_signature() {
    let source = format!("{APPLY} func main() -> i32 {{ apply(|x| x + 1, 41) }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "apply");
    let call = tail(&hir, body);
    assert!(matches!(
        results.call(call),
        Some(ResolvedCall::FnTrait {
            kind: ClosureKind::Fn,
            ..
        })
    ));
}

#[test]
fn twice_calls_the_bound_more_than_once() {
    let source = format!("{APPLY} func main() -> i32 {{ let base = 1; twice(move || base + 1) }}");
    check_clean(&source);
}

#[test]
fn each_requires_a_mutable_binding_and_calls_fn_mut() {
    let source =
        format!("{APPLY} func main() {{ let mut total = 0; each(|x| total = total + x); }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "each");
    let call = expr_statement(
        &hir,
        *statements(&hir, body).last().expect("each has a call"),
    );
    assert!(matches!(
        results.call(call),
        Some(ResolvedCall::FnTrait {
            kind: ClosureKind::FnMut,
            ..
        })
    ));
}

#[test]
fn an_fn_mut_bound_needs_a_mutable_binding() {
    let source = "\
        func each<F: FnMut(i32)>(f: F) { f(1); } \
        func main() { each(|x| { }); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3017", "not declared as mutable");
}

#[test]
fn consume_calls_the_bound_once() {
    let source =
        format!("{APPLY} func main() -> i32 {{ let base = 1; consume(move || base + 1) }}");
    check_clean(&source);
}

#[test]
fn an_fn_bound_accepts_a_closure_and_a_safe_function_pointer() {
    let source = format!(
        "{APPLY} func increment(value: i32) -> i32 {{ value + 1 }} \
         func main() -> i32 {{ apply(increment, 41) + apply(|x| x * 2, 20) }}"
    );
    check_clean(&source);
}

#[test]
fn fn_mut_accepts_an_fn_closure_by_supertrait() {
    let source = "\
        func each<F: FnMut(i32)>(f: F, value: i32) { let mut f = f; f(value); } \
        func main() { each(|x| { }, 1); }";
    check_clean(source);
}

#[test]
fn fn_once_accepts_all_three_closure_kinds() {
    let source = "\
        func consume<F: FnOnce() -> i32>(f: F) -> i32 { f() } \
        func main() -> i32 { \
            let base = 1; \
            let shared = || base; \
            let mut total = 0; \
            let exclusive = || { total = total + 1; base }; \
            consume(move || base) + consume(shared) + consume(exclusive) \
        }";
    check_clean(source);
}

#[test]
fn an_fn_bound_rejects_an_fn_mut_closure() {
    let source = "\
        func want<F: Fn() -> i32>(f: F) -> i32 { f() } \
        func main() -> i32 { \
            let mut total = 0; \
            want(|| { total = total + 1; total }) \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "requires `Fn`");
    fixture.expect_help("E3096", "relax the bound");
}

#[test]
fn a_non_copy_fn_once_parameter_cannot_be_called_twice() {
    let source = "\
        func twice<F: FnOnce() -> i32>(f: F) -> i32 { f() + f() } \
        func main() -> i32 { twice(|| 1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3011", "was moved");
}

#[test]
fn a_copy_fn_once_parameter_may_be_called_twice() {
    let source = "\
        func zero() -> i32 { 0 } \
        func twice<F: FnOnce() -> i32 + Copy>(f: F) -> i32 { f() + f() } \
        func main() -> i32 { twice(zero) }";
    check_clean(source);
}

#[test]
fn a_concrete_fn_once_closure_cannot_be_called_twice() {
    let source = "\
        struct Token { value: i32 } \
        func take(token: Token) -> i32 { token.value } \
        func main() { \
            let token = Token { value: 1 }; \
            let consume = move || take(token); \
            consume(); \
            consume(); \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3011", "was moved");
}

#[test]
fn a_closure_parameter_without_a_signature_bound_is_e3094() {
    let source = "\
        func take<F: Copy>(f: F) { } \
        func main() { take(|x| x); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3094", "cannot infer the type of the closure parameter");
}

#[test]
fn an_fn_bound_rejects_an_fn_once_closure() {
    let source = "\
        struct Token { value: i32 } \
        func take(token: Token) -> i32 { token.value } \
        func want<F: Fn() -> i32>(f: F) -> i32 { f() } \
        func main() -> i32 { \
            let token = Token { value: 1 }; \
            want(move || take(token)) \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "requires `Fn`");
}

#[test]
fn a_safe_bound_rejects_an_unsafe_function_pointer() {
    let source = "\
        unsafe func increment(value: i32) -> i32 { value + 1 } \
        func want<F: Fn(i32) -> i32>(f: F) -> i32 { f(1) } \
        func main() -> i32 { want(increment) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "unsafe func");
}

#[test]
fn a_where_clause_supplies_the_same_bound() {
    let source = "\
        func apply<F>(f: F, value: i32) -> i32 where F: Fn(i32) -> i32 { f(value) } \
        func main() -> i32 { apply(|x| x + 1, 41) }";
    check_clean(source);
}

#[test]
fn the_explicit_tuple_spelling_is_equivalent_to_the_sugar() {
    let source = "\
        func apply<F: Fn<(i32,), i32>>(f: F, value: i32) -> i32 { f(value) } \
        func main() -> i32 { apply(|x| x + 1, 41) }";
    check_clean(source);
}

#[test]
fn a_call_with_the_wrong_arity_is_rejected() {
    let source = "\
        func apply<F: Fn(i32) -> i32>(f: F) -> i32 { f(1, 2) } \
        func main() -> i32 { apply(|x| x + 1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3008", "1 argument");
}

#[test]
fn fn_once_accepts_a_safe_function_pointer() {
    let source = "\
        func zero() -> i32 { 0 } \
        func consume<F: FnOnce() -> i32>(f: F) -> i32 { f() } \
        func main() -> i32 { consume(zero) }";
    check_clean(source);
}

#[test]
fn two_compatible_bounds_on_one_parameter_are_accepted() {
    let source = "\
        func apply<F>(f: F, value: i32) -> i32 \
        where F: Fn(i32) -> i32 + FnMut(i32) -> i32 { f(value) } \
        func main() -> i32 { apply(|x| x + 1, 41) }";
    check_clean(source);
}

#[test]
fn two_incompatible_fn_bounds_are_rejected() {
    let source = "\
        func apply<F>(f: F) where F: Fn(i32) -> i32 + Fn(i64) -> i64 { f(1); } \
        func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "disagree on the callable signature");
}

#[test]
fn a_non_tuple_first_argument_is_rejected() {
    let source = "func apply<F: Fn<i32, i32>>(f: F) { } func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "must be a tuple");
    fixture.expect_help("E3096", "Fn(A, B) -> R");
}

#[test]
fn a_wrong_argument_count_is_rejected() {
    let source = "func apply<F: Fn<i32>>(f: F) { } func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "takes two type arguments");
    fixture.expect_help("E3096", "Fn(A, B) -> R");
}

#[test]
fn a_function_trait_bound_without_arguments_is_rejected() {
    let source = "func apply<F: Fn>(f: F) { } func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "needs argument and result types");
    fixture.expect_help("E3096", "Fn(A, B) -> R");
}

#[test]
fn an_unsized_function_result_is_rejected() {
    let source = "func apply<F: Fn() -> str>(f: F) { } func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "must be sized");
}

#[test]
fn declaring_a_function_trait_is_e3097() {
    let source = "trait Fn<Args, Output> { func call_once(self: Self, args: Args) -> Output; } func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3097", "reserved");
}

#[test]
fn implementing_a_function_trait_is_e3097() {
    let source = "\
        struct Callable { value: i32 } \
        impl Fn<(i32,), i32> for Callable { \
            func call(self: *Self, args: (i32,)) -> i32 { self.value } \
        } \
        func main() { }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3097", "cannot be implemented explicitly");
}

#[test]
fn a_dyn_function_trait_is_rejected_without_an_ice() {
    let source = "\
        func main() { \
            let pointer: *dyn Fn(i32) -> i32 = 0 as *dyn Fn(i32) -> i32; \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "cannot be used as a trait object");
    fixture.expect_help("E3096", "generic bound");
}

#[test]
fn supertraits_make_fn_prove_fn_mut_and_fn_once() {
    check_clean(
        "\
        func need_mut<F: FnMut(i32) -> i32>(f: F) -> i32 { let mut f = f; f(1) } \
        func need_once<F: FnOnce(i32) -> i32>(f: F) -> i32 { f(1) } \
        func pass<F: Fn(i32) -> i32>(f: F) -> i32 { need_mut(f) + need_once(f) } \
        func main() -> i32 { pass(|x| x + 1) }",
    );
}

#[test]
fn a_weaker_bound_does_not_prove_a_stronger_one() {
    let source = "\
        func need_fn<F: Fn(i32) -> i32>(f: F) -> i32 { f(1) } \
        func pass<F: FnMut(i32) -> i32>(f: F) -> i32 { need_fn(f) } \
        func main() -> i32 { pass(|x| x + 1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3039", "not satisfied");
}

#[test]
fn a_call_method_on_a_bound_value_is_still_e3096() {
    let source = "\
        func apply<F: Fn(i32) -> i32>(f: F) -> i32 { f.call(1) } \
        func main() -> i32 { apply(|x| x + 1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3096", "function-trait bound");
    fixture.expect_help("E3096", "f(...)");
}

#[test]
fn an_unannotated_closure_infers_from_the_bound() {
    let source = format!("{APPLY} func main() -> i32 {{ apply(|x| x + 1, 41) }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "apply");
    let call = tail(&hir, body);
    assert!(matches!(
        results.call(call),
        Some(ResolvedCall::FnTrait {
            kind: ClosureKind::Fn,
            ..
        })
    ));
    let ExprKind::Call { args, .. } = &hir.expr(call).kind else {
        panic!("the tail is not a call");
    };
    let parameter = args[0];
    assert!(matches!(hir.expr(parameter).kind, ExprKind::Path { .. }));
}
