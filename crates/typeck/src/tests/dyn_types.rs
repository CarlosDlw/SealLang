//! Trait-object coercions, reborrows, and dynamic method lookup.

use seal_hir::{ExprKind, Hir, HirExprId, HirStmtId, StmtKind};
use seal_layout::VtableLayout;
use seal_ty::RefKind;

use crate::{Coercion, MethodKind, ReceiverAdjust};

use super::support::{
    check_clean, check_errors, expr_statement, expr_ty, function, let_binding, local_ty,
    statements, tail, ty_text_with,
};

const SHAPE: &str = "\
struct Circle { radius: i32 } \
trait Shape { func area(self: *Self) -> i32; } \
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } }";

const BUMP: &str = "\
struct Counter { value: i32 } \
trait Bump { func bump(self: *mut Self); } \
impl Bump for Counter { func bump(self: *mut Counter) { (*self).value += 1; } }";

fn only_statement_expr(hir: &Hir, body: &seal_hir::Body) -> HirExprId {
    expr_statement(hir, statements(hir, body)[0])
}

fn let_init(hir: &Hir, statement: HirStmtId) -> HirExprId {
    match hir.stmt(statement).kind.clone() {
        StmtKind::Let {
            init: Some(init), ..
        } => init,
        other => panic!("expected an initialized let, got {other:?}"),
    }
}

#[test]
fn shared_pointer_coerces_to_shared_dyn() {
    let source = format!("{SHAPE} func upcast(circle: *Circle) -> *dyn Shape {{ circle }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "upcast");
    let value = tail(&hir, body);
    let coercion = results.coercion(value).expect("the return coerces");
    let Coercion::TraitObject { source, target } = coercion else {
        panic!("expected a trait-object coercion");
    };
    assert_eq!(ty_text_with(&fixture, &hir, source), "*Circle");
    assert_eq!(ty_text_with(&fixture, &hir, target), "*dyn Shape");
    assert_eq!(expr_ty(&results, value), target);
}

#[test]
fn exclusive_pointer_coerces_to_exclusive_dyn() {
    let source =
        format!("{BUMP} func upcast(counter: *mut Counter) -> *mut dyn Bump {{ counter }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "upcast");
    let value = tail(&hir, body);
    let Some(Coercion::TraitObject { target, .. }) = results.coercion(value) else {
        panic!("expected a trait-object coercion");
    };
    assert_eq!(ty_text_with(&fixture, &hir, target), "*mut dyn Bump");
}

#[test]
fn repeated_coercions_use_the_same_target_type() {
    let source = format!(
        "{SHAPE} func upcast(first: *Circle, second: *Circle) {{ \
           let a: *dyn Shape = first; \
           let b: *dyn Shape = second; \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "upcast");
    let statements = statements(&hir, body);
    let first = let_init(&hir, statements[0]);
    let second = let_init(&hir, statements[1]);
    let Some(Coercion::TraitObject { target: a, .. }) = results.coercion(first) else {
        panic!("the first let coerces");
    };
    let Some(Coercion::TraitObject { target: b, .. }) = results.coercion(second) else {
        panic!("the second let coerces");
    };
    assert_eq!(a, b);
    assert_eq!(ty_text_with(&fixture, &hir, a), "*dyn Shape");
}

#[test]
fn coercion_works_in_let_assignment_argument_return_field_tuple_and_array() {
    let source = format!(
        "{SHAPE} \
         struct Holder {{ shape: *dyn Shape }} \
         func take(shape: *dyn Shape) {{ }} \
         func make(circle: *Circle) -> *dyn Shape {{ circle }} \
         func sites(circle: *Circle, other: *Circle) {{ \
           let mut assigned: *dyn Shape = circle; \
           assigned = other; \
           take(circle); \
           let pair: (*dyn Shape,) = (circle,); \
           let list: [*dyn Shape; 1] = [circle]; \
           let holder = Holder {{ shape: circle }}; \
           let _ = make(circle); \
         }}"
    );
    check_clean(&source);
}

#[test]
fn coercion_works_through_parenthesized_and_block_expressions() {
    let source = format!(
        "{SHAPE} func parenthesized(circle: *Circle) -> *dyn Shape {{ (circle) }} \
         func blocked(circle: *Circle) -> *dyn Shape {{ {{ circle }} }}"
    );
    check_clean(&source);
}

#[test]
fn coercion_works_in_if_and_match_arms() {
    let source = format!(
        "{SHAPE} \
         func choose(flag: bool, first: *Circle, second: *Circle) -> *dyn Shape {{ \
           if flag {{ first }} else {{ second }} \
         }} \
         func choose_match(flag: bool, first: *Circle, second: *Circle) -> *dyn Shape {{ \
           match flag {{ true => first, false => second }} \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);

    let (_, body) = function(&fixture, &hir, "choose");
    let ExprKind::If {
        then_branch,
        else_branch,
        ..
    } = hir.expr(tail(&hir, body)).kind.clone()
    else {
        panic!("expected an if");
    };
    for branch in [then_branch, else_branch.expect("the if has an else")] {
        let value = hir.block(branch).tail.expect("the branch has a tail");
        assert!(matches!(
            results.coercion(value),
            Some(Coercion::TraitObject { .. })
        ));
    }

    let (_, body) = function(&fixture, &hir, "choose_match");
    let ExprKind::Match { arms, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a match");
    };
    for arm in arms {
        assert!(matches!(
            results.coercion(arm.body),
            Some(Coercion::TraitObject { .. })
        ));
    }
}

#[test]
fn generic_bound_coerces_to_a_trait_object() {
    let source = format!("{SHAPE} func upcast<T: Shape>(value: *T) -> *dyn Shape {{ value }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "upcast");
    let value = tail(&hir, body);
    assert!(matches!(
        results.coercion(value),
        Some(Coercion::TraitObject { .. })
    ));
}

#[test]
fn mutability_is_preserved_by_coercion() {
    let source =
        format!("{BUMP} func upcast(counter: *mut Counter) -> *mut dyn Bump {{ counter }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "upcast");
    let Some(Coercion::TraitObject { target, .. }) = results.coercion(tail(&hir, body)) else {
        panic!("the return coerces");
    };
    let (kind, pointee) = hir.ty_ctxt().ref_parts(target).expect("a pointer target");
    assert_eq!(kind, RefKind::Exclusive);
    assert!(matches!(
        hir.ty_ctxt().kind(pointee),
        seal_ty::TyKind::Dyn { .. }
    ));
}

#[test]
fn missing_impl_coercion_is_e3046() {
    let source = "trait Other { } struct Point { x: i32 } \
                  func upcast(point: *Point) -> *dyn Other { point }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3046", "does not implement");
}

#[test]
fn coercion_with_wrong_impl_arguments_is_e3046() {
    let source = "\
        trait Convert<T> { } \
        struct Point { x: i32 } \
        impl Convert<i32> for Point { } \
        func upcast(point: *Point) -> *dyn Convert<bool> { point }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3046", "does not implement");
}

#[test]
fn nested_coercion_is_e3047() {
    let source = "\
        trait Root { } \
        trait Leaf: Root { } \
        func upcast(value: *dyn Leaf) -> *dyn Root { value }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3047", "cannot coerce one trait-object pointer");
}

#[test]
fn mut_to_shared_dyn_is_rejected() {
    let source = format!("{SHAPE} func upcast(circle: *mut Circle) -> *dyn Shape {{ circle }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3047", "reference kinds differ");
}

#[test]
fn bare_dyn_value_is_e3010() {
    let source = format!("{SHAPE} func f() {{ let object: dyn Shape; }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "unsized");
}

#[test]
fn bare_dyn_parameter_and_return_are_e3010() {
    let source =
        format!("{SHAPE} func take(object: dyn Shape) {{ }} func make() -> dyn Shape {{ }}");
    let (fixture, _, _) = check_errors(&source);
    assert!(
        fixture
            .codes()
            .iter()
            .filter(|code| *code == "E3010")
            .count()
            >= 2
    );
}

#[test]
fn bare_dyn_field_and_array_element_are_e3010() {
    let source = format!(
        "{SHAPE} struct Holder {{ shape: dyn Shape }} \
         func f() {{ let values: [dyn Shape; 1]; }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "unsized");
}

#[test]
fn dyn_as_a_generic_argument_is_e3010() {
    let source = format!(
        "{SHAPE} struct Holder<T> {{ value: T }} \
         func f() {{ let holder: Holder<dyn Shape>; }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "unsized");
}

#[test]
fn dyn_as_a_trait_argument_is_e3010() {
    let source = format!("{SHAPE} trait Boxed<T> {{ }} func f<T: Boxed<dyn Shape>>() {{ }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "unsized");
}

#[test]
fn dyn_as_an_explicit_type_argument_is_e3010() {
    let source = format!(
        "{SHAPE} func take<T>(value: *T) {{ }} \
         func f(value: *dyn Shape) {{ take::<dyn Shape>(value); }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "type argument");
}

#[test]
fn dyn_inferred_as_a_type_argument_is_e3010() {
    let source = format!(
        "{SHAPE} func take<T>(value: *T) {{ }} \
         func f(value: *dyn Shape) {{ take(value); }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "type argument");
}

#[test]
fn dyn_as_a_trait_method_bound_argument_is_e3010() {
    let source = format!(
        "{SHAPE} trait Boxed<T> {{ }} trait Holder {{ func take<T: Boxed<dyn Shape>>(self: *Self); }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "trait argument");
}

#[test]
fn dyn_as_a_supertrait_argument_is_e3010() {
    let source = format!("{SHAPE} trait Boxed<T> {{ }} trait Holder: Boxed<dyn Shape> {{ }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3010", "trait argument");
}

#[test]
fn pointer_to_dyn_can_be_a_struct_field() {
    let source = format!("{SHAPE} struct Holder {{ shape: *dyn Shape }}");
    check_clean(&source);
}

#[test]
fn pointer_to_dyn_can_be_a_generic_argument() {
    let source = format!(
        "{SHAPE} struct Holder<T> {{ value: T }} \
         func f() {{ let holder: Holder<*dyn Shape>; }}"
    );
    check_clean(&source);
}

#[test]
fn pointer_to_dyn_can_be_returned() {
    let source = format!("{SHAPE} func keep(value: *dyn Shape) -> *dyn Shape {{ value }}");
    check_clean(&source);
}

#[test]
fn inference_does_not_invent_a_trait_object() {
    let source = format!("{SHAPE} func f(circle: *Circle) {{ let value = circle; }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "f");
    let statement = statements(&hir, body)[0];
    assert_eq!(
        ty_text_with(
            &fixture,
            &hir,
            local_ty(&results, let_binding(&hir, statement)),
        ),
        "*Circle"
    );
}

#[test]
fn shared_method_call_on_dyn() {
    let source = format!("{SHAPE} func area(object: *dyn Shape) -> i32 {{ object.area() }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "area");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("the dynamic call is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::Dynamic {
            receiver: seal_hir::ReceiverKind::Shared,
            ..
        }
    ));
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::None);
    assert_eq!(resolved.arg_tys.len(), 0);
}

#[test]
fn exclusive_method_call_on_mut_dyn() {
    let source = format!("{BUMP} func bump(object: *mut dyn Bump) {{ object.bump(); }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "bump");
    let call = only_statement_expr(&hir, body);
    let resolved = results.method(call).expect("the dynamic call is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::Dynamic {
            receiver: seal_hir::ReceiverKind::Exclusive,
            ..
        }
    ));
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::None);
}

#[test]
fn exclusive_method_on_shared_dyn_is_e3029() {
    let source = format!("{BUMP} func bump(object: *dyn Bump) {{ object.bump(); }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3029", "needs exclusive access");
}

#[test]
fn supertrait_method_call_on_dyn() {
    let source = "\
        trait Root { func root(self: *Self) -> i32; } \
        trait Leaf: Root { } \
        func read(value: *dyn Leaf) -> i32 { value.root() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "read");
    let call = tail(&hir, body);
    let resolved = results
        .method(call)
        .expect("the supertrait method resolves");
    let MethodKind::Dynamic {
        object_trait_ref, ..
    } = resolved.kind
    else {
        panic!("expected dynamic dispatch");
    };
    assert_eq!(
        object_trait_ref.trait_def,
        hir.trait_def(fixture.interner().intern("Leaf"))
            .expect("Leaf exists")
    );
}

#[test]
fn principal_method_wins_over_a_supertrait_method() {
    let source = "\
        trait Root { func name(self: *Self) -> i32; } \
        trait Leaf: Root { func name(self: *Self) -> i32; } \
        func read(value: *dyn Leaf) -> i32 { value.name() }";
    let (fixture, hir, results) = check_clean(source);
    let root = hir
        .trait_def(fixture.interner().intern("Root"))
        .expect("Root exists");
    let leaf = hir
        .trait_def(fixture.interner().intern("Leaf"))
        .expect("Leaf exists");
    let leaf_method = hir
        .trait_definition(leaf)
        .and_then(|definition| {
            definition
                .method(fixture.interner().intern("name"))
                .cloned()
        })
        .expect("Leaf::name exists");
    let root_method = hir
        .trait_definition(root)
        .and_then(|definition| {
            definition
                .method(fixture.interner().intern("name"))
                .cloned()
        })
        .expect("Root::name exists");
    let (_, body) = function(&fixture, &hir, "read");
    let resolved = results
        .method(tail(&hir, body))
        .expect("the dynamic call resolves");
    let MethodKind::Dynamic {
        method,
        object_trait_ref,
        ..
    } = resolved.kind
    else {
        panic!("expected dynamic dispatch");
    };
    assert_eq!(method, leaf_method.def);
    assert_ne!(method, root_method.def);
    assert_eq!(object_trait_ref.trait_def, leaf);
}

#[test]
fn ambiguous_supertrait_method_is_e2028() {
    let source = "\
        trait Left { func size(self: *Self) -> i32; } \
        trait Right { func size(self: *Self) -> i32; } \
        trait Both: Left + Right { } \
        func read(value: *dyn Both) -> i32 { value.size() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2028", "declared by more than one trait");
}

#[test]
fn unknown_dyn_method_is_e3027() {
    let source = format!("{SHAPE} func read(value: *dyn Shape) -> i32 {{ value.missing() }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3027", "no method named `missing`");
}

#[test]
fn static_method_call_on_dyn_is_e2019() {
    let source = "\
        trait Shape { func unit() -> i32; } \
        func read(value: *dyn Shape) -> i32 { value.unit() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2019", "associated function");
}

#[test]
fn mut_dyn_reborrows_shared_for_a_shared_method() {
    let source = format!("{SHAPE} func area(object: *mut dyn Shape) -> i32 {{ object.area() }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "area");
    let resolved = results
        .method(tail(&hir, body))
        .expect("the dynamic call resolves");
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::SharedReborrow);
}

#[test]
fn shared_reborrow_of_mut_dyn_is_allowed() {
    let source = format!(
        "{SHAPE} func take(value: *dyn Shape) -> i32 {{ value.area() }} \
         func read(value: *mut dyn Shape) -> i32 {{ let shared: *dyn Shape = &*value; take(&*value) }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "read");
    let statement = statements(&hir, body)[0];
    let borrow = let_init(&hir, statement);
    assert!(matches!(
        results.coercion(borrow),
        Some(Coercion::TraitObjectReborrow {
            source,
            target
        }) if source != target
    ));
}

#[test]
fn exclusive_reborrow_of_shared_dyn_is_e3017() {
    let source = format!("{SHAPE} func read(value: *dyn Shape) {{ let exclusive = &mut *value; }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3017", "shared reference");
}

#[test]
fn shared_reborrow_works_in_let_and_argument_position() {
    let source = format!(
        "{SHAPE} func take(value: *dyn Shape) -> i32 {{ value.area() }} \
         func read(value: *mut dyn Shape) -> i32 {{ take(&*value) }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "read");
    let call = tail(&hir, body);
    let ExprKind::Call { args, .. } = hir.expr(call).kind.clone() else {
        panic!("expected a call");
    };
    assert!(matches!(
        results.coercion(args[0]),
        Some(Coercion::TraitObjectReborrow { .. })
    ));
}

#[test]
fn method_call_through_explicit_deref() {
    let source = format!("{SHAPE} func area(object: *mut dyn Shape) -> i32 {{ (*object).area() }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "area");
    let resolved = results
        .method(tail(&hir, body))
        .expect("the dynamic call resolves");
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::SharedBorrow);
}

#[test]
fn exclusive_method_call_through_explicit_deref() {
    let source = format!("{BUMP} func bump(object: *mut dyn Bump) {{ (*object).bump(); }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "bump");
    let resolved = results
        .method(only_statement_expr(&hir, body))
        .expect("the dynamic call resolves");
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::ExclusiveBorrow);
}

#[test]
fn method_slot_matches_vtable_layout() {
    let source = format!("{SHAPE} func area(object: *dyn Shape) -> i32 {{ object.area() }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "area");
    let call = tail(&hir, body);
    let ExprKind::MethodCall { receiver, .. } = hir.expr(call).kind.clone() else {
        panic!("expected a method call");
    };
    let receiver_ty = expr_ty(&results, receiver);
    let (_, object) = hir
        .ty_ctxt()
        .ref_parts(receiver_ty)
        .expect("the receiver is a pointer");
    let layout = VtableLayout::for_object(hir.ty_ctxt(), object).expect("the vtable lays out");
    let resolved = results.method(call).expect("the dynamic call resolves");
    let MethodKind::Dynamic { method, slot, .. } = resolved.kind else {
        panic!("expected dynamic dispatch");
    };
    assert_eq!(layout.slot_of(method), Some(slot));
}

#[test]
fn dynamic_call_records_no_concrete_instance() {
    let source = format!("{SHAPE} func area(object: *dyn Shape) -> i32 {{ object.area() }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "area");
    let call = tail(&hir, body);
    assert!(results.method(call).is_some());
    assert!(results.instantiation(call).is_none());
}
