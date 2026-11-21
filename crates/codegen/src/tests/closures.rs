//! IR shape of function pointers, closures, and their trampolines.

use seal_def::DefKind;
use seal_ty::Instance;

use super::support::{
    Compiled, front_end, function_ir, lowered_debug_module_ir, lowered_module_ir,
};

/// Returns every closure instance in collection order.
fn closures(compiled: &Compiled) -> Vec<Instance> {
    compiled
        .mono
        .instances()
        .iter()
        .map(|mono| mono.instance)
        .filter(|instance| compiled.hir.definitions().kind(instance.def) == DefKind::Closure)
        .collect()
}

/// Returns the linker symbol of one instance.
fn symbol(compiled: &Compiled, instance: Instance) -> String {
    crate::backend::symbol(&compiled.hir, compiled.session.interner(), instance)
}

/// Returns the IR of the only closure body in a fixture.
fn closure_body_ir(compiled: &Compiled, ir: &str) -> String {
    let instances = closures(compiled);
    assert_eq!(instances.len(), 1, "the fixture has exactly one closure");
    function_ir(ir, &symbol(compiled, instances[0]))
}

#[test]
fn a_function_value_uses_the_internal_abi() {
    let compiled = front_end(
        "struct Pair { left: i32, right: i32 } \
         func sum(pair: Pair) -> i32 { pair.left + pair.right } \
         func main() -> i32 { \
             let f: func(Pair) -> i32 = sum; \
             f(Pair { left: 20, right: 22 }) \
         }",
    );
    let ir = lowered_module_ir(&compiled);
    let main = function_ir(&ir, "seal.main");
    assert!(
        main.contains("store ptr @seal.sum"),
        "the function value is the internal symbol:\n{main}"
    );
    assert!(
        main.contains("fn pointer call") && main.contains("call i32 %"),
        "the call goes through the loaded pointer:\n{main}"
    );
    assert!(
        !main.contains("sret") && !main.contains("byval"),
        "the aggregate uses the internal ABI:\n{main}"
    );
    let sum = function_ir(&ir, "seal.sum");
    assert!(
        sum.starts_with("define hidden i32 @seal.sum(%seal.adt."),
        "the target takes the aggregate directly:\n{sum}"
    );
    assert!(
        !ir.contains("seal.callback"),
        "a Seal function value never emits a C trampoline"
    );
}

#[test]
fn a_capture_free_closure_calls_through_an_internal_trampoline() {
    let compiled = front_end("func main() -> i32 { let f: func(i32) -> i32 = |x| x + 1; f(41) }");
    let ir = lowered_module_ir(&compiled);
    assert!(
        ir.contains("store ptr @seal.closure.main."),
        "the closure coerces to its trampoline:\n{ir}"
    );
    assert!(
        !ir.contains("seal.callback"),
        "a closure used as a Seal function pointer never emits a C trampoline"
    );
    let body = closure_body_ir(&compiled, &ir);
    assert!(
        body.starts_with("define hidden i32 @seal.main.closure(ptr readonly %0, i32 %1)"),
        "the lifted body still takes the environment:\n{body}"
    );
    let trampoline = ir
        .lines()
        .find(|line| line.starts_with("define internal i32 @seal.closure.main."))
        .expect("the trampoline is defined");
    assert!(
        trampoline.ends_with("(i32 %0) {"),
        "the trampoline drops the environment parameter: {trampoline}"
    );
    assert!(
        ir.contains("@seal.main.closure(ptr %closure.environment, i32 %0)"),
        "the trampoline builds an empty environment and calls the body:\n{ir}"
    );
}

#[test]
fn an_fn_environment_is_read_only() {
    let compiled =
        front_end("func main() -> i32 { let value = 40; let add = || value + 2; add() }");
    let ir = lowered_module_ir(&compiled);
    assert!(
        ir.contains("%seal.adt.0.closure = type { ptr }"),
        "the environment stores one shared reference:\n{ir}"
    );
    let body = closure_body_ir(&compiled, &ir);
    assert!(
        body.starts_with("define hidden i32 @seal.main.closure(ptr readonly %0)"),
        "an `Fn` body reads a shared environment:\n{body}"
    );
    assert!(
        body.contains("getelementptr inbounds nuw %seal.adt.0.closure, ptr %dereference"),
        "the capture is reached through the environment field:\n{body}"
    );
}

#[test]
fn an_fn_mut_environment_is_exclusive() {
    let compiled = front_end(
        "func main() -> i32 { \
             let mut total = 0; \
             let mut add = |value: i32| { total += value; }; \
             add(2); \
             total \
         }",
    );
    let ir = lowered_module_ir(&compiled);
    assert!(
        ir.contains("%seal.adt.0.closure = type { ptr }"),
        "the environment stores one exclusive reference:\n{ir}"
    );
    let body = closure_body_ir(&compiled, &ir);
    let signature = body.lines().next().expect("the body has a signature");
    assert!(
        signature.contains("@seal.main.closure(ptr %0, i32 %1)"),
        "an `FnMut` body takes an exclusive environment: {signature}"
    );
    assert!(
        !body.contains("ptr readonly"),
        "the exclusive environment is not declared read-only:\n{body}"
    );
}

#[test]
fn an_fn_once_environment_is_passed_by_value() {
    let compiled = front_end(
        "struct Token { value: i32 } \
         func main() -> i32 { \
             let token = Token { value: 42 }; \
             let take = move || token; \
             let consumed = take(); \
             consumed.value \
         }",
    );
    let ir = lowered_module_ir(&compiled);
    let body = closure_body_ir(&compiled, &ir);
    assert!(
        body.starts_with("define hidden %seal.adt."),
        "an `FnOnce` body returns its captured value:\n{body}"
    );
    let signature = body.lines().next().expect("the body has a signature");
    assert!(
        signature.contains("(%seal.adt."),
        "the environment is passed by value: {signature}"
    );
    assert!(
        !signature.contains("ptr"),
        "an `FnOnce` body has no environment pointer: {signature}"
    );
}

#[test]
fn debug_info_names_a_closure_and_its_captures() {
    let compiled = front_end(
        "struct Token { value: i32 } \
         func main() -> i32 { \
             let token = Token { value: 42 }; \
             let take = move || token; \
             let consumed = take(); \
             consumed.value \
         }",
    );
    let ir = lowered_debug_module_ir(&compiled);
    assert!(
        ir.matches("<closure>").count() >= 2,
        "the body and the environment both use the closure name:\n{ir}"
    );
    assert!(
        ir.contains("tag: DW_TAG_member, name: \"token\""),
        "the capture keeps the source name in debug information:\n{ir}"
    );
}

#[test]
fn a_closure_environment_records_each_capture_mode() {
    let compiled = front_end(
        "func main() -> i32 { \
             let value = 40; \
             let read = || value; \
             let copied = move || value; \
             read() + copied() - 38 \
         }",
    );
    let ir = lowered_module_ir(&compiled);
    assert_eq!(closures(&compiled).len(), 2, "the fixture has two closures");
    assert!(
        ir.contains("%seal.adt.0.closure = type { ptr }")
            && ir.contains("%seal.adt.1.closure = type { i32 }"),
        "one environment borrows and one owns the capture:\n{ir}"
    );
    assert!(
        ir.matches("insertvalue %seal.adt.0.closure").count() == 1
            && ir.matches("insertvalue %seal.adt.1.closure").count() == 1,
        "each closure builds exactly one environment:\n{ir}"
    );
}
