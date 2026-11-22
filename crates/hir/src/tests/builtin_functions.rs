//! The compiler-backed standard-library function classifier.

use super::support::{Fixture, PackageSpec, expr_statement, function, lower_clean};
use crate::{BuiltinFunction, ExprKind, Hir, StmtKind};
use seal_def::DefId;

fn function_named(hir: &Hir, interner: &seal_symbol::Interner, name: &str) -> Option<DefId> {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| interner.resolve(data.name) == name)
        .map(|(id, _)| id)
}

#[test]
fn a_user_function_named_forget_is_not_builtin() {
    let (fixture, hir) = lower_clean("func forget(value: i32) -> i32 { value }");
    let function = function_named(&hir, &fixture.interner, "forget").expect("function");
    assert_eq!(hir.builtin_function(function, &fixture.interner), None);
}

#[test]
fn a_module_named_mem_in_a_user_package_is_not_builtin() {
    let (fixture, hir) = lower_clean("module mem;\nfunc forget(value: i32) -> i32 { value }");
    let function = function_named(&hir, &fixture.interner, "forget").expect("function");
    assert_eq!(hir.builtin_function(function, &fixture.interner), None);
}

fn app_with_core(main: &'static str) -> Fixture {
    let files: &'static [(&'static str, &'static str)] = Box::leak(Box::new([("main.sl", main)]));
    Fixture::from_packages(&[
        PackageSpec {
            name: "core",
            files: &[(
                "mem.sl",
                "module mem;\npublic func forget<T>(value: T) { __forget(value); }",
            )],
            exposed: &["mem"],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files,
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ])
}

#[test]
fn a_direct_core_mem_forget_call_lowers_to_a_dedicated_node() {
    let fixture = app_with_core(
        "use core::mem::forget;\n\
         struct Pair { value: i32 }\n\
         func main() { let pair = Pair { value: 1 }; forget(pair); }",
    );
    let hir = fixture.lower_packages();
    fixture.assert_clean();
    let (_, body) = function(&fixture, &hir, "main");
    let call = expr_statement(&hir, hir.block(body.root).statements[1]);
    assert!(matches!(hir.expr(call).kind, ExprKind::Forget { .. }));
}

#[test]
fn an_imported_alias_lowers_to_the_same_node() {
    let fixture = app_with_core(
        "use core::mem::forget as release;\n\
         struct Pair { value: i32 }\n\
         func main() { let pair = Pair { value: 1 }; release(pair); }",
    );
    let hir = fixture.lower_packages();
    fixture.assert_clean();
    let (_, body) = function(&fixture, &hir, "main");
    let call = expr_statement(&hir, hir.block(body.root).statements[1]);
    assert!(matches!(hir.expr(call).kind, ExprKind::Forget { .. }));
}

#[test]
fn a_forget_call_with_explicit_type_arguments_stays_a_call() {
    let fixture = app_with_core(
        "use core::mem::forget;\n\
         struct Pair { value: i32 }\n\
         func main() { let pair = Pair { value: 1 }; forget::<Pair>(pair); }",
    );
    let hir = fixture.lower_packages();
    fixture.assert_clean();
    let (_, body) = function(&fixture, &hir, "main");
    let call = expr_statement(&hir, hir.block(body.root).statements[1]);
    assert!(matches!(hir.expr(call).kind, ExprKind::Call { .. }));
}

#[test]
fn forget_used_as_a_value_stays_a_function_item() {
    let fixture = app_with_core(
        "use core::mem::forget;\n\
         func main() { let consume = forget::<i32>; consume(1); }",
    );
    let hir = fixture.lower_packages();
    fixture.assert_clean();
    let (_, body) = function(&fixture, &hir, "main");
    let value = match hir.stmt(hir.block(body.root).statements[0]).kind {
        StmtKind::Let {
            init: Some(value), ..
        } => value,
        ref other => panic!("expected a let with an initializer, got {other:?}"),
    };
    assert!(matches!(hir.expr(value).kind, ExprKind::Path { .. }));
    let call = expr_statement(&hir, hir.block(body.root).statements[1]);
    assert!(matches!(hir.expr(call).kind, ExprKind::Call { .. }));
}

#[test]
fn the_core_wrapper_body_uses_the_private_builtin() {
    let fixture = app_with_core("func main() {}");
    let hir = fixture.lower_packages();
    fixture.assert_clean();
    let wrapper = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| {
            data.kind == seal_def::DefKind::Function && fixture.symbol(data.name) == "forget"
        })
        .map(|(id, _)| id)
        .expect("the core wrapper exists");
    let body = hir.body_of(wrapper).expect("the wrapper has a body");
    let call = expr_statement(&hir, hir.block(body.root).statements[0]);
    assert!(matches!(hir.expr(call).kind, ExprKind::Forget { .. }));
}

#[test]
fn a_direct_forget_call_requires_exactly_one_argument() {
    let fixture = app_with_core("use core::mem::forget;\nfunc main() { forget(); }");
    let _ = fixture.lower_packages();
    fixture.expect("E3008", "takes exactly one argument");
}

#[test]
fn the_canonical_core_mem_forget_is_builtin() {
    let fixture = Fixture::from_packages(&[PackageSpec {
        name: "core",
        files: &[(
            "mem.sl",
            "module mem;\npublic func forget(value: i32) -> i32 { value }",
        )],
        exposed: &["mem"],
        dependencies: &[],
    }]);
    let hir = fixture.lower_packages();
    let function = function_named(&hir, &fixture.interner, "forget").expect("function");
    assert_eq!(
        hir.builtin_function(function, &fixture.interner),
        Some(BuiltinFunction::Forget)
    );
}
