//! The compiler-provided function traits and their bound validation.

use seal_ty::{ClosureKind, RefKind, TraitMethodReceiver};

use crate::Hir;

use super::support::{Fixture, lower_clean};

fn trait_definition(hir: &Hir, kind: ClosureKind) -> seal_ty::TraitDef {
    hir.trait_definition(hir.fn_trait(kind))
        .unwrap_or_else(|| panic!("{kind} is installed"))
}

fn method<'a>(
    fixture: &Fixture,
    definition: &'a seal_ty::TraitDef,
    name: &str,
) -> &'a seal_ty::TraitMethodDef {
    let symbol = fixture.interner.intern(name);
    definition
        .methods
        .iter()
        .find(|method| method.name == symbol)
        .unwrap_or_else(|| panic!("{name} is declared"))
}

#[test]
fn the_three_function_traits_are_installed_with_methods_and_supertraits() {
    let (fixture, hir) = lower_clean("func main() -> i32 { 0 }");
    let cases = [
        (ClosureKind::Fn, "call"),
        (ClosureKind::FnMut, "call_mut"),
        (ClosureKind::FnOnce, "call_once"),
    ];
    for (kind, method_name) in cases {
        let definition = trait_definition(&hir, kind);
        assert_eq!(definition.generics.params.len(), 3);
        assert!(definition.generics.params[0].is_self);
        assert!(hir.is_builtin_def(definition.def));
        assert_eq!(
            hir.fn_trait_named(fixture.interner.intern(kind.as_str())),
            Some(kind)
        );
        assert_eq!(hir.fn_trait_kind(definition.def), Some(kind));
        assert_eq!(hir.fn_trait_method(kind), definition.methods[0].def);

        let method = method(&fixture, &definition, method_name);
        assert!(!method.has_body);
        assert!(hir.is_builtin_def(method.def));
        assert_eq!(method.signature.params.len(), 1);
        assert_param(&hir, &definition, method.signature.params[0], 1);
        assert_param(&hir, &definition, method.signature.ret, 2);
    }
}

#[test]
fn function_trait_receivers_follow_the_closure_kind() {
    let (fixture, hir) = lower_clean("func main() -> i32 { 0 }");
    let cases = [
        (ClosureKind::Fn, "call", Some(RefKind::Shared)),
        (ClosureKind::FnMut, "call_mut", Some(RefKind::Exclusive)),
        (ClosureKind::FnOnce, "call_once", None),
    ];
    for (kind, name, reference) in cases {
        let definition = trait_definition(&hir, kind);
        let method = method(&fixture, &definition, name);
        assert_eq!(method.receiver, Some(TraitMethodReceiver { reference }));
    }
}

#[test]
fn the_supertrait_chain_is_fn_to_fn_mut_to_fn_once() {
    let (_, hir) = lower_clean("func main() -> i32 { 0 }");
    let fn_ = trait_definition(&hir, ClosureKind::Fn);
    let fn_mut = trait_definition(&hir, ClosureKind::FnMut);
    let fn_once = trait_definition(&hir, ClosureKind::FnOnce);

    assert!(fn_once.supertraits.is_empty());
    assert_eq!(fn_mut.supertraits.len(), 1);
    assert_eq!(fn_mut.supertraits[0].trait_def, fn_once.def);
    assert_eq!(fn_.supertraits.len(), 1);
    assert_eq!(fn_.supertraits[0].trait_def, fn_mut.def);
}

#[test]
fn the_sugar_lowers_to_a_tuple_and_result_argument() {
    let (fixture, hir) = lower_clean(
        "func apply<F: Fn(i32, bool) -> i32>(f: F) -> i32 { 0 } \
         func empty<F: Fn() -> bool>(f: F) -> bool { true } \
         func main() -> i32 { 0 }",
    );
    fixture.assert_clean();
    let apply = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| fixture.symbol(data.name) == "apply")
        .map(|(id, _)| id)
        .expect("apply exists");
    let bound = hir.generics(apply).expect("apply is generic").params[0].bounds[0];
    let seal_ty::Bound::Trait(trait_ref) = bound else {
        panic!("the bound is a trait");
    };
    assert_eq!(
        hir.fn_trait_kind(trait_ref.trait_def),
        Some(ClosureKind::Fn)
    );
    let args = hir.ty_ctxt().substs_args(trait_ref.args);
    assert_eq!(args.len(), 2);
    assert!(matches!(
        hir.ty_ctxt().kind(args[0]),
        seal_ty::TyKind::Tuple(_)
    ));

    let empty = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| fixture.symbol(data.name) == "empty")
        .map(|(id, _)| id)
        .expect("empty exists");
    let bound = hir.generics(empty).expect("empty is generic").params[0].bounds[0];
    let seal_ty::Bound::Trait(trait_ref) = bound else {
        panic!("the bound is a trait");
    };
    let args = hir.ty_ctxt().substs_args(trait_ref.args);
    assert!(matches!(hir.ty_ctxt().kind(args[0]), seal_ty::TyKind::Unit));
}

#[test]
fn malformed_function_trait_bounds_are_e3096() {
    let cases = [
        (
            "func f<F: Fn>(f: F) {} func main() {}",
            "needs argument and result types",
        ),
        (
            "func f<F: Fn<i32>>(f: F) {} func main() {}",
            "takes two type arguments",
        ),
        (
            "func f<F: Fn<i32, i32>>(f: F) {} func main() {}",
            "must be a tuple",
        ),
        (
            "func f<F: Fn() -> str>(f: F) {} func main() {}",
            "must be sized",
        ),
    ];
    for (source, message) in cases {
        let fixture = Fixture::new(source);
        let _ = fixture.lower();
        fixture.expect("E3096", message);
    }
}

#[test]
fn declaring_or_implementing_a_function_trait_is_e3097() {
    let declaration = Fixture::new("trait Fn<A, R> { } func main() {}");
    let _ = declaration.lower();
    declaration.expect("E3097", "reserved");

    let implementation = Fixture::new(
        "struct C { value: i32 } \
         impl Fn<(i32,), i32> for C { func call(self: *Self, args: (i32,)) -> i32 { 0 } } \
         func main() {}",
    );
    let _ = implementation.lower();
    implementation.expect("E3097", "cannot be implemented explicitly");
}

#[test]
fn a_dyn_function_trait_remains_e3096() {
    let fixture =
        Fixture::new("func main() { let value: *dyn Fn() -> i32 = 0 as *dyn Fn() -> i32; }");
    let _ = fixture.lower();
    fixture.expect("E3096", "cannot be used as a trait object");
    assert!(
        !fixture.codes().iter().any(|code| code == "E2065"),
        "object-safety validation must not run for a function trait"
    );
}

#[test]
fn no_synthetic_implementation_is_created() {
    let (_, hir) = lower_clean(
        "func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) } \
         func main() -> i32 { apply(|x| x + 1, 41) }",
    );
    assert!(hir.trait_impls().is_empty());
}

fn assert_param(hir: &Hir, definition: &seal_ty::TraitDef, ty: seal_ty::TyId, index: usize) {
    assert_eq!(
        hir.ty_ctxt().kind(ty),
        seal_ty::TyKind::Param(definition.generics.params[index].param),
        "the type is parameter {index}"
    );
}
