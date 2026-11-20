//! Collection order, specialization, and limits.

use seal_def::{DefId, DefKind};
use seal_mir::{CallTarget, Rvalue, StatementKind};
use seal_ty::{Instance, IntTy};

use crate::collector::Limits;
use crate::tests::support::{Fixture, collect_clean, collect_errors, names};
use crate::{MonoProgram, instance_hash, instance_name};

/// Returns the collected instance rendered as `name`.
fn instance_named<'a>(
    fixture: &Fixture,
    hir: &seal_hir::Hir,
    program: &'a MonoProgram,
    name: &str,
) -> &'a crate::MonoMir {
    for instance in program.instances() {
        if instance_name(hir, fixture.interner(), instance.instance) == name {
            return instance;
        }
    }
    panic!(
        "no instance named `{name}`, have {:?}",
        names(fixture, hir, program)
    )
}

fn def_named(fixture: &Fixture, hir: &seal_hir::Hir, kind: DefKind, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

/// Returns every direct call target in `body`, in statement order.
fn call_targets(body: &seal_mir::Body) -> Vec<CallTarget> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                return None;
            };
            match rvalue {
                Rvalue::Call { target, .. } => Some(*target),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn a_function_address_keeps_its_target_reachable() {
    let (fixture, hir, _mir, program) = collect_clean(
        r#"
        func double_it(value: i32) -> i32 { value * 2 }
        func main() -> i32 {
            unsafe {
                let callback = double_it as *raw extern "c" func(i32) -> i32;
                0
            }
        }"#,
    );
    let mut collected = names(&fixture, &hir, &program);
    collected.sort();
    assert_eq!(collected, ["double_it", "main"]);
}

#[test]
fn a_non_generic_program_collects_one_instance_per_function() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func helper(value: i32) -> i32 { value }\
         func main() -> i32 { helper(1) }",
    );
    assert_eq!(program.instances().len(), 2);
    assert_eq!(names(&fixture, &hir, &program), ["helper", "main"]);
    for (index, instance) in program.instances().iter().enumerate() {
        assert_eq!(
            program
                .id_of(instance.instance)
                .expect("known instance")
                .index() as usize,
            index,
            "IDs are dense and follow discovery order"
        );
    }
}

#[test]
fn a_concrete_call_collects_only_the_reachable_instance() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() -> i32 { identity(1) }",
    );
    assert_eq!(names(&fixture, &hir, &program), ["main", "identity<i32>"]);
}

#[test]
fn distinct_type_arguments_create_distinct_instances() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() { identity(1); identity(true); }",
    );
    assert_eq!(
        names(&fixture, &hir, &program),
        ["main", "identity<i32>", "identity<bool>"]
    );
}

#[test]
fn repeated_calls_reuse_one_instance() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() { identity(1); identity(2); }",
    );
    assert_eq!(program.instances().len(), 2);
    assert_eq!(names(&fixture, &hir, &program), ["main", "identity<i32>"]);
}

#[test]
fn nested_arguments_are_substituted_through_the_chain() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func inner<T>(value: T) -> T { value }\
         func outer<T>(first: T, second: T) -> [T; 2] { inner([first, second]) }\
         func main() -> i32 { let values = outer(1, 2); values[0] }",
    );
    let rendered = names(&fixture, &hir, &program);
    assert!(
        rendered.iter().any(|name| name.contains("[i32; 2]")),
        "expected inner<[i32; 2]>, have {rendered:?}"
    );
}

#[test]
fn signatures_and_locals_use_the_instance_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Wrapper<T> { value: T }\
         func wrap<T>(value: T) -> Wrapper<T> { Wrapper { value: value } }\
         func main() -> i32 { let wrapped = wrap(1); wrapped.value }",
    );
    let wrapped = instance_named(&fixture, &hir, &program, "wrap<i32>");
    let return_local = &wrapped.body.locals[seal_mir::Body::return_local()];
    assert_eq!(
        hir.ty_ctxt()
            .display_with(return_local.ty, fixture.interner())
            .to_string(),
        "Wrapper<i32>"
    );
}

#[test]
fn generic_enum_types_are_substituted_in_signatures_and_variants() {
    let (fixture, hir, _mir, program) = collect_clean(
        "enum Maybe<T> { Empty, Full(T) }\
         func wrap<T>(value: T) -> Maybe<T> { Maybe::Full(value) }\
         func main() { wrap(1); }",
    );
    let wrapped = instance_named(&fixture, &hir, &program, "wrap<i32>");
    let return_local = &wrapped.body.locals[seal_mir::Body::return_local()];
    assert_eq!(
        hir.ty_ctxt()
            .display_with(return_local.ty, fixture.interner())
            .to_string(),
        "Maybe<i32>"
    );
    let arguments = wrapped
        .body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                return None;
            };
            let Rvalue::Aggregate {
                kind: seal_mir::AggregateKind::Variant { args, .. },
                ..
            } = rvalue
            else {
                return None;
            };
            Some(hir.ty_ctxt().substs_args(*args))
        })
        .expect("wrap builds the variant");
    assert_eq!(
        arguments
            .iter()
            .map(|argument| hir
                .ty_ctxt()
                .display_with(*argument, fixture.interner())
                .to_string())
            .collect::<Vec<_>>(),
        ["i32"]
    );
}

#[test]
fn a_trait_method_calling_another_trait_method_resolves_both() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Pair { first: i32, second: i32 }\
         trait B { func b(self: *Self) -> i32; }\
         trait A { func a(self: *Self) -> i32; }\
         impl B for Pair { func b(self: *Pair) -> i32 { (*self).first } }\
         impl A for Pair { func a(self: *Pair) -> i32 { (*self).b() } }\
         func apply<T: A>(value: *T) -> i32 { value.a() }\
         func main() -> i32 { let pair = Pair { first: 1, second: 2 }; apply(&pair) }",
    );
    let rendered = names(&fixture, &hir, &program);
    assert!(rendered.contains(&"a".to_owned()), "have {rendered:?}");
    assert!(rendered.contains(&"b".to_owned()), "have {rendered:?}");
    let apply = instance_named(&fixture, &hir, &program, "apply<Pair>");
    assert!(matches!(
        call_targets(&apply.body).first(),
        Some(CallTarget::Direct { .. })
    ));
    let a = instance_named(&fixture, &hir, &program, "a");
    let target = call_targets(&a.body).into_iter().next().expect("a calls b");
    assert!(matches!(target, CallTarget::Direct { .. }));
}

#[test]
fn aggregate_kinds_carry_the_instance_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Wrapper<T> { value: T }\
         func wrap<T>(value: T) -> Wrapper<T> { Wrapper { value: value } }\
         func main() -> i32 { let wrapped = wrap(1); wrapped.value }",
    );
    let wrapped = instance_named(&fixture, &hir, &program, "wrap<i32>");
    let arguments = wrapped
        .body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                return None;
            };
            let Rvalue::Aggregate {
                kind: seal_mir::AggregateKind::Struct { args, .. },
                ..
            } = rvalue
            else {
                return None;
            };
            Some(hir.ty_ctxt().substs_args(*args))
        })
        .expect("wrap builds the struct");
    assert_eq!(
        arguments
            .iter()
            .map(|argument| hir
                .ty_ctxt()
                .display_with(*argument, fixture.interner())
                .to_string())
            .collect::<Vec<_>>(),
        ["i32"]
    );
}

#[test]
fn a_generic_method_instance_carries_impl_and_method_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Wrapper<T> { value: T }\
         impl<T> Wrapper<T> {\
             func map<U>(self: *Wrapper<T>, other: U) -> U { other }\
         }\
         func main() -> bool { let wrapped = Wrapper { value: 1 }; wrapped.map(true) }",
    );
    let map = instance_named(&fixture, &hir, &program, "map<i32, bool>");
    assert_eq!(
        hir.definitions().kind(map.instance.def),
        DefKind::Function,
        "the instance names the impl method"
    );
}

#[test]
fn a_generic_associated_function_keeps_its_type_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Holder<T> { value: T }\
         impl<T> Holder<T> { func keep(value: T) -> T { value } }\
         func main() -> i32 { Holder::keep(1) }",
    );
    assert!(
        names(&fixture, &hir, &program)
            .iter()
            .any(|name| name == "keep<i32>")
    );
}

#[test]
fn a_symbolic_trait_call_resolves_to_the_impl_method() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Show { func show(self: *Self) -> i32; }\
         impl Show for i32 { func show(self: *i32) -> i32 { *self } }\
         func show_one<T: Show>(value: *T) -> i32 { value.show() }\
         func main() -> i32 { let value = 1; show_one(&value) }",
    );
    let show_one = instance_named(&fixture, &hir, &program, "show_one<i32>");
    let target = call_targets(&show_one.body)
        .into_iter()
        .next()
        .expect("show_one calls the bound method");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the symbolic call was not resolved");
    };
    assert_eq!(instance_name(&hir, fixture.interner(), instance), "show");
    assert!(
        program.id_of(instance).is_some(),
        "the selected impl method is collected"
    );
}

#[test]
fn a_generic_default_method_keeps_its_own_type_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Item { value: i32 }\
         trait Echo { func echo<U>(self: *Self, value: U) -> U { value } }\
         impl Echo for Item {}\
         func main() -> bool {\
             let item = Item { value: 1 };\
             item.echo::<bool>(true)\
         }",
    );
    let main = instance_named(&fixture, &hir, &program, "main");
    let target = call_targets(&main.body)
        .into_iter()
        .next()
        .expect("main calls echo");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the generic default was not resolved");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::TraitMethod);
    let arguments: Vec<String> = hir
        .ty_ctxt()
        .substs_args(instance.args)
        .iter()
        .map(|argument| {
            hir.ty_ctxt()
                .display_with(*argument, fixture.interner())
                .to_string()
        })
        .collect();
    assert_eq!(arguments, ["Item", "bool"]);
    assert!(program.id_of(instance).is_some());
}

#[test]
fn a_generic_trait_method_keeps_its_own_type_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Convert { func convert<U>(self: *Self, value: U) -> U; }\
         impl Convert for i32 { func convert<U>(self: *i32, value: U) -> U { value } }\
         func use_it<T: Convert, U>(value: *T, other: U) -> U { value.convert(other) }\
         func main() -> bool { let value = 1; use_it(&value, true) }",
    );
    let use_it = instance_named(&fixture, &hir, &program, "use_it<i32, bool>");
    let target = call_targets(&use_it.body)
        .into_iter()
        .next()
        .expect("use_it calls the trait method");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the generic trait call was not resolved");
    };
    assert_eq!(
        instance_name(&hir, fixture.interner(), instance),
        "convert<bool>",
        "the impl has no parameters of its own, so only the method argument remains"
    );
    assert!(program.id_of(instance).is_some());
}

#[test]
fn a_conditional_impl_is_selected_only_for_satisfying_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Pair<T> { first: T, second: T }\
         trait Mark { func mark(self: *Self) -> i32; }\
         impl<T: Copy> Mark for Pair<T> { func mark(self: *Self) -> i32 { 0 } }\
         func apply<T: Mark>(value: *T) -> i32 { value.mark() }\
         func main() -> i32 { let value = Pair { first: 1, second: 2 }; apply(&value) }",
    );
    let apply = instance_named(&fixture, &hir, &program, "apply<Pair<i32>>");
    assert!(matches!(
        call_targets(&apply.body).first(),
        Some(CallTarget::Direct { .. })
    ));
}

#[test]
fn a_supertrait_method_resolves_through_the_leaf_bound() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Root { func root(self: *Self) -> i32; }\
         trait Leaf: Root { }\
         impl Root for i32 { func root(self: *i32) -> i32 { *self } }\
         impl Leaf for i32 { }\
         func read<T: Leaf>(value: *T) -> i32 { value.root() }\
         func main() -> i32 { let value = 1; read(&value) }",
    );
    let read = instance_named(&fixture, &hir, &program, "read<i32>");
    let target = call_targets(&read.body)
        .into_iter()
        .next()
        .expect("the supertrait method is called");
    assert!(matches!(target, CallTarget::Direct { .. }));
}

#[test]
fn exact_recursion_terminates_with_one_instance() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func f<T>(value: T) { f(value); }\
         func main() { f(1); }",
    );
    assert_eq!(names(&fixture, &hir, &program), ["main", "f<i32>"]);
}

#[test]
fn mutual_recursion_terminates_with_one_instance_each() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func f<T>(value: T) { g(value); }\
         func g<T>(value: T) { f(value); }\
         func main() { f(1); }",
    );
    assert_eq!(
        names(&fixture, &hir, &program),
        ["main", "f<i32>", "g<i32>"]
    );
}

#[test]
fn growing_recursion_reports_the_instance_chain() {
    let (fixture, _hir, _mir, _program) = collect_errors(
        "func grow<T>(value: T) { grow([value]); }\
         func main() { grow(0); }",
    );
    let diagnostic = fixture.expect("E7001", "recursion limit");
    assert!(
        diagnostic.labels().len() > 1,
        "the diagnostic lists the path, got {:?}",
        diagnostic.labels()
    );
}

#[test]
fn growing_template_recursion_hits_the_limit_deterministically() {
    let source = "struct Pair<A, B> { first: A, second: B }\
                  func grow<T>(value: T) { grow(Pair { first: value, second: 0 }); }\
                  func main() { grow(0); }";
    let (first, _hir, _mir, _program) = collect_errors(source);
    let (second, _hir, _mir, _program) = collect_errors(source);
    let code = |fixture: &Fixture| {
        fixture
            .diagnostics()
            .iter()
            .find_map(|diagnostic| diagnostic.code().map(|code| code.as_str().to_owned()))
    };
    assert_eq!(code(&first), Some("E7001".to_owned()));
    assert_eq!(code(&first), code(&second));
}

#[test]
fn the_instance_limit_is_enforced_before_allocation() {
    let fixture =
        Fixture::new("func a() { b(); } func b() { c(); } func c() { } func main() { a(); }");
    let (hir, _mir, program) = fixture.lower_with(Limits {
        depth: 64,
        instances: 2,
        vtables: 4096,
        glues: 4096,
        glue_depth: 64,
    });
    fixture.expect("E7004", "instance limit");
    assert_eq!(program.instances().len(), 2);
    assert!(!hir.bodies().is_empty());
}

#[test]
fn discovery_order_and_ids_are_stable_across_collections() {
    let source = "func helper<T>(value: T) -> T { value }\
                  func main() { helper(1); helper(true); }";
    let (first_for_fixture, first_hir, _mir, first) = collect_clean(source);
    let (second_for_fixture, second_hir, _mir, second) = collect_clean(source);
    let first_names = names(&first_for_fixture, &first_hir, &first);
    let second_names = names(&second_for_fixture, &second_hir, &second);
    assert_eq!(first_names, second_names);
    for (index, instance) in first.instances().iter().enumerate() {
        let id = first.id_of(instance.instance).expect("collected");
        assert_eq!(id.index() as usize, index);
    }
}

#[test]
fn an_uncalled_generic_function_is_not_emitted() {
    let (fixture, hir, _mir, program) =
        collect_clean("func unused<T>(value: T) { } func main() { }");
    assert_eq!(names(&fixture, &hir, &program), ["main"]);
}

#[test]
fn a_generic_function_called_from_dead_code_is_emitted() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func never() { identity(1); }\
         func main() { }",
    );
    assert!(
        names(&fixture, &hir, &program).contains(&"identity<i32>".to_owned()),
        "every non-generic function is a root"
    );
}

#[test]
fn a_generic_entry_point_is_rejected() {
    let (fixture, _hir, _mir, program) = collect_errors("func main<T>() { }");
    fixture.expect("E7003", "entry point cannot be generic");
    assert!(program.is_empty());
}

#[test]
fn instance_symbols_differ_by_nested_type_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() { let value = 1; identity(1); identity([1, 2]); identity(&value); }",
    );
    let identity: Vec<(String, _)> = program
        .instances()
        .iter()
        .map(|instance| {
            (
                instance_name(&hir, fixture.interner(), instance.instance),
                instance_hash(&hir, instance.instance),
            )
        })
        .filter(|(name, _)| name.starts_with("identity<"))
        .collect();
    assert!(identity.len() >= 3, "expected three instantiations");
    for right in 0..identity.len() {
        for left in 0..right {
            assert_ne!(
                identity[left].1, identity[right].1,
                "{} and {} must hash differently",
                identity[left].0, identity[right].0
            );
        }
    }
}

#[test]
fn the_stable_hash_is_equal_across_fresh_collections() {
    let source = "func identity<T>(value: T) -> T { value }\
                  func main() { identity(1); identity([1, 2]); }";
    let (first_fixture, first_hir, _mir, first) = collect_clean(source);
    let (second_fixture, second_hir, _mir, second) = collect_clean(source);
    for instance in first.instances() {
        if let Some(counterpart) = second.instances().iter().find(|candidate| {
            candidate.instance.def == instance.instance.def
                && candidate.instance.args == instance.instance.args
        }) {
            assert_eq!(
                instance_hash(&first_hir, instance.instance),
                instance_hash(&second_hir, counterpart.instance)
            );
        }
    }
    let _ = (&first_fixture, &second_fixture);
}

#[test]
fn a_bound_call_uses_the_default_when_the_impl_omits_the_method() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Show { func show(self: *Self) -> i32 { 7 } }\
         impl Show for i32 {}\
         func apply<T: Show>(value: *T) -> i32 { value.show() }\
         func main() -> i32 { let value = 1; apply(&value) }",
    );
    let apply = instance_named(&fixture, &hir, &program, "apply<i32>");
    let target = call_targets(&apply.body)
        .into_iter()
        .next()
        .expect("apply calls the bound method");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the default call was not resolved");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::TraitMethod);
    assert!(program.id_of(instance).is_some());
    program
        .validate(&hir)
        .expect("the default instance is valid");
}

#[test]
fn a_concrete_omitted_method_uses_the_default() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Circle { radius: i32 }\
         trait Shape { func area(self: *Self) -> i32 { 7 } }\
         impl Shape for Circle {}\
         func main() -> i32 {\
             let circle = Circle { radius: 1 };\
             circle.area()\
         }",
    );
    let main = instance_named(&fixture, &hir, &program, "main");
    let target = call_targets(&main.body)
        .into_iter()
        .next()
        .expect("main calls area");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the default call was not resolved");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::TraitMethod);
    assert!(program.id_of(instance).is_some());
    program
        .validate(&hir)
        .expect("the default instance is valid");
}

#[test]
fn a_qualified_associated_default_is_monomorphized() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Circle { radius: i32 }\
         trait Factory { func make() -> i32 { 7 } }\
         impl Factory for Circle {}\
         func main() -> i32 { Circle::make() }",
    );
    let main = instance_named(&fixture, &hir, &program, "main");
    let target = call_targets(&main.body)
        .into_iter()
        .next()
        .expect("main calls make");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the associated default was not resolved");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::TraitMethod);
    let arguments: Vec<String> = hir
        .ty_ctxt()
        .substs_args(instance.args)
        .iter()
        .map(|argument| {
            hir.ty_ctxt()
                .display_with(*argument, fixture.interner())
                .to_string()
        })
        .collect();
    assert_eq!(arguments, ["Circle"]);
    assert!(program.id_of(instance).is_some());
    program
        .validate(&hir)
        .expect("the associated default instance is valid");
}

#[test]
fn an_override_still_wins_over_a_default() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Circle { radius: i32 }\
         trait Shape { func area(self: *Self) -> i32 { 7 } }\
         impl Shape for Circle { func area(self: *Circle) -> i32 { 8 } }\
         func main() -> i32 {\
             let circle = Circle { radius: 1 };\
             circle.area()\
         }",
    );
    let main = instance_named(&fixture, &hir, &program, "main");
    let target = call_targets(&main.body)
        .into_iter()
        .next()
        .expect("main calls area");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the override call was not resolved");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::Function);
}

#[test]
fn a_supertrait_default_resolves_from_the_declaring_trait() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Root { func root(self: *Self) -> i32 { 5 } }\
         trait Leaf: Root {}\
         impl Root for i32 {}\
         impl Leaf for i32 {}\
         func read<T: Leaf>(value: *T) -> i32 { value.root() }\
         func main() -> i32 { let value = 1; read(&value) }",
    );
    let read = instance_named(&fixture, &hir, &program, "read<i32>");
    let target = call_targets(&read.body)
        .into_iter()
        .next()
        .expect("read calls root");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the supertrait default was not resolved");
    };
    let root = def_named(&fixture, &hir, DefKind::Trait, "Root");
    assert_eq!(hir.definitions().get(instance.def).parent, Some(root));
    assert_eq!(hir.definitions().kind(instance.def), DefKind::TraitMethod);
}

#[test]
fn a_generic_default_keeps_self_and_trait_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct Wrapper<T> { value: T }\
         trait Convert<T> {\
             func value(self: *Self) -> T;\
             func twice(self: *Self) -> T { self.value() }\
         }\
         impl Convert<i32> for Wrapper<i32> {\
             func value(self: *Wrapper<i32>) -> i32 { (*self).value }\
         }\
         func main() -> i32 {\
             let wrapper = Wrapper { value: 4 };\
             wrapper.twice()\
         }",
    );
    let main = instance_named(&fixture, &hir, &program, "main");
    let target = call_targets(&main.body)
        .into_iter()
        .next()
        .expect("main calls twice");
    let CallTarget::Direct { instance, .. } = target else {
        panic!("the generic default was not resolved");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::TraitMethod);
    let arguments: Vec<String> = hir
        .ty_ctxt()
        .substs_args(instance.args)
        .iter()
        .map(|argument| {
            hir.ty_ctxt()
                .display_with(*argument, fixture.interner())
                .to_string()
        })
        .collect();
    assert_eq!(arguments, ["Wrapper<i32>", "i32"]);
    program
        .validate(&hir)
        .expect("the generic default is valid");
}

#[test]
fn a_default_methods_own_bounds_are_validated() {
    let fixture = Fixture::new(
        "struct Token { value: i32 }\
         trait Show { func show<U: Copy>(self: *Self) -> i32 { 7 } }\
         impl Show for i32 {}",
    );
    let (hir, _mir) = fixture.frontend();
    let show = def_named(&fixture, &hir, DefKind::TraitMethod, "show");
    let token = def_named(&fixture, &hir, DefKind::Struct, "Token");
    let token = hir.adt(token).expect("Token is an ADT");
    let arguments = [
        hir.ty_ctxt().int_ty(IntTy::I32),
        hir.ty_ctxt().adt_ty(token),
    ];
    let instance = Instance::new(show, hir.ty_ctxt().substs(&arguments));
    let error = crate::substitute::resolve_trait_instance(&hir, fixture.interner(), instance)
        .expect_err("Token is not Copy");
    assert!(matches!(error, crate::MonoError::MissingBound { .. }));
}

#[test]
fn a_recursive_default_collects_one_instance() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Loop { func f(self: *Self) -> i32 { self.f() } }\
         impl Loop for i32 {}\
         func main() -> i32 { let value = 1; value.f() }",
    );
    assert_eq!(
        names(&fixture, &hir, &program)
            .iter()
            .filter(|name| name.starts_with("f<"))
            .count(),
        1
    );
    program.validate(&hir).expect("recursive default is valid");
}

#[test]
fn a_growing_recursive_default_hits_the_existing_limit() {
    let fixture = Fixture::new(
        "trait Grow {\
             func grow<T>(self: *Self, value: T) -> i32 { self.grow([value]) }\
         }\
         impl Grow for i32 {}\
         func main() -> i32 { let value = 1; value.grow(0) }",
    );
    let (_hir, _mir, program) = fixture.lower_with(Limits {
        depth: 4,
        instances: 4096,
        vtables: 4096,
        glues: 4096,
        glue_depth: 64,
    });
    fixture.expect("E7001", "recursion limit");
    assert!(!program.instances().is_empty());
}

#[test]
fn a_missing_impl_is_reported_not_an_ice() {
    // The generic function is never instantiated, so type checking has no
    // concrete type to reject and resolution runs with no impl to select.
    let fixture = Fixture::new(
        "trait Show { func show(self: *Self) -> i32; }\
         func show_one<T: Show>(value: *T) -> i32 { value.show() }",
    );
    let (hir, mir) = fixture.frontend();
    let owner = mir
        .bodies()
        .iter()
        .find(|body| fixture.symbol(hir.definitions().name(body.owner)) == "show_one")
        .expect("show_one has a body")
        .owner;
    let body = mir.body_of(owner).expect("show_one was lowered");
    let target = call_targets(body)
        .into_iter()
        .next()
        .expect("show_one calls the bound method");
    let CallTarget::TraitMethod { instance, .. } = target else {
        panic!("a bound call stays symbolic");
    };
    let error = crate::substitute::resolve_trait_instance(&hir, fixture.interner(), instance)
        .expect_err("no impl matches");
    assert!(matches!(error, crate::MonoError::MissingImpl { .. }));
}

#[test]
fn one_hundred_distinct_instances_are_collected_once_each() {
    let mut source =
        String::from("func mark<T>() -> i32 { 1 }\nfunc main() -> i32 {\n  let mut total = 0;\n");
    for length in 1..=100 {
        source.push_str(&format!("  total = total + mark::<[i32; {length}]>();\n"));
    }
    source.push_str("  total\n}\n");
    let (fixture, hir, _mir, program) = collect_clean(&source);
    let rendered = names(&fixture, &hir, &program);
    assert_eq!(
        rendered.len(),
        101,
        "one `main` plus one instance per distinct array length"
    );
    assert_eq!(
        rendered
            .iter()
            .filter(|name| name.starts_with("mark<"))
            .count(),
        100
    );
}

#[test]
fn many_repeated_calls_still_collect_one_instance() {
    let mut source =
        String::from("func mark<T>() -> i32 { 1 }\nfunc main() -> i32 {\n  let mut total = 0;\n");
    for _ in 0..100 {
        source.push_str("  total = total + mark::<i32>();\n");
    }
    source.push_str("  total\n}\n");
    let (fixture, hir, _mir, program) = collect_clean(&source);
    assert_eq!(names(&fixture, &hir, &program), ["main", "mark<i32>"]);
}

#[test]
fn a_symbolic_instance_keeps_its_trait_method_definition() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Show { func show(self: *Self) -> i32; }\
         impl Show for i32 { func show(self: *i32) -> i32 { *self } }\
         func show_one<T: Show>(value: *T) -> i32 { value.show() }\
         func main() -> i32 { let value = 1; show_one(&value) }",
    );
    let show_one = instance_named(&fixture, &hir, &program, "show_one<i32>");
    let targets = call_targets(&show_one.body);
    let Some(CallTarget::Direct { instance, .. }) = targets.first() else {
        panic!("the call is direct");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::Function);
    let _: Instance = *instance;
}
