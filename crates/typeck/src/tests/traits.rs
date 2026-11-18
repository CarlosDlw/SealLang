//! Trait method dispatch, ambiguity, and shared reborrow.

use seal_def::{DefId, DefKind};
use seal_hir::ExprKind;

use crate::{MethodKind, ReceiverAdjust};

use super::support::{check_clean, check_errors, function, tail};

const SHAPE: &str = "\
struct Circle { radius: i32 } \
trait Shape { func area(self: *Self) -> i32; } \
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius * (*self).radius } }";

#[test]
fn a_trait_impl_for_a_primitive_provides_a_callable_method() {
    let source = "\
        trait Show { func show(self: *Self) -> i32; } \
        impl Show for i32 { func show(self: *i32) -> i32 { *self } } \
        func main() -> i32 { 7.show() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("show is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Shared,
            ..
        }
    ));
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::SharedBorrow);
}

#[test]
fn an_implemented_trait_wins_over_the_intrinsic_catalog() {
    let source = "\
        trait Size { func len(self: *Self) -> bool; } \
        impl Size for [i32; 2] { func len(self: *[i32; 2]) -> bool { true } } \
        func main() -> bool { \
            let values = [1, 2]; \
            values.len() \
        }";
    check_clean(source);
}

#[test]
fn a_generic_trait_impl_matches_the_self_arguments() {
    let source = "\
        trait Keyed<K> { func key(self: *Self) -> K; } \
        struct Pair<T, U> { first: T, second: U } \
        impl<K> Keyed<K> for Pair<K, K> { \
            func key(self: *Pair<K, K>) -> K { (*self).first } \
        } \
        func main() -> i32 { \
            let pair = Pair { first: 1, second: 2 }; \
            pair.key() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("key is resolved");
    assert!(matches!(resolved.kind, MethodKind::Trait { .. }));
    assert_eq!(resolved.arg_tys.len(), 0);
}

#[test]
fn a_partially_fixed_trait_impl_matches_only_compatible_types() {
    let source = "\
        trait Keyed<K> { func key(self: *Self) -> K; } \
        struct Pair<T, U> { first: T, second: U } \
        impl<T> Keyed<i32> for Pair<T, i32> { \
            func key(self: *Pair<T, i32>) -> i32 { (*self).second } \
        } \
        func good() -> i32 { Pair { first: true, second: 1 }.key() } \
        func bad() -> i32 { Pair { first: 1, second: true }.key() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3027", "no method named `key`");
}

#[test]
fn a_bound_associated_function_records_its_symbolic_instantiation() {
    let source = "\
        trait Factory { func make() -> i32; } \
        func build<T: Factory>() -> i32 { T::make() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "build");
    let call = tail(&hir, body);
    let ExprKind::Call { callee, .. } = hir.expr(call).kind.clone() else {
        panic!("expected a call");
    };
    let ExprKind::Path {
        res: seal_hir::Res::Function(function),
        ..
    } = hir.expr(callee).kind
    else {
        panic!("expected a function path");
    };
    let instance = results.instantiation(call).expect("instantiation recorded");
    assert_eq!(instance.def, function);
    assert_eq!(hir.ty_ctxt().substs_args(instance.args).len(), 1);
}

#[test]
fn a_generic_trait_method_infers_its_own_parameter_for_a_concrete_impl() {
    let source = "\
        trait Convert { func convert<U>(self: *Self, value: U) -> U; } \
        impl Convert for i32 { func convert<U>(self: *i32, value: U) -> U { value } } \
        func main() -> bool { 1.convert(true) }";
    check_clean(source);
}

#[test]
fn a_generic_trait_method_infers_its_own_parameter_through_a_bound() {
    let source = "\
        trait Convert { func convert<U>(self: *Self, value: U) -> U; } \
        func pass<T: Convert, U>(value: *T, input: U) -> U { value.convert(input) }";
    check_clean(source);
}

#[test]
fn a_trait_impl_for_a_reference_is_callable_on_a_reference() {
    let source = "\
        trait Show { func show(self: *Self) -> i32; } \
        impl Show for *i32 { func show(self: **i32) -> i32 { **self } } \
        func main() -> i32 { \
            let value = 7; \
            let pointer = &value; \
            pointer.show() \
        }";
    check_clean(source);
}

#[test]
fn a_generic_trait_impl_respects_value_and_exclusive_receivers() {
    let source = "\
        struct Wrapper<T> { value: T } \
        trait Take { func take(self: Self) -> i32; } \
        trait Reset { func reset(self: *mut Self); } \
        impl<T> Take for Wrapper<T> { func take(self: Wrapper<T>) -> i32 { 1 } } \
        impl<T> Reset for Wrapper<T> { \
            func reset(self: *mut Wrapper<T>) { } \
        } \
        func main() -> i32 { \
            let mut wrapper = Wrapper { value: 0 }; \
            wrapper.reset(); \
            wrapper.take() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let take = tail(&hir, body);
    assert!(matches!(
        results.method(take).expect("take is resolved").kind,
        MethodKind::Trait { .. }
    ));
}

#[test]
fn bound_checks_do_not_depend_on_call_order() {
    let source = "\
        trait Show { } \
        struct First { } struct Second { } \
        impl Show for First { } impl Show for Second { } \
        func need<T: Show>(value: T) { } \
        func forward() { need(First { }); need(Second { }); } \
        func backward() { need(Second { }); need(First { }); }";
    check_clean(source);
}

#[test]
fn trait_impls_cover_bool_and_tuples() {
    let source = "\
        trait Flag { func flag(self: *Self) -> bool; } \
        trait First { func first(self: *Self) -> i32; } \
        impl Flag for bool { func flag(self: *bool) -> bool { *self } } \
        impl First for (i32, bool) { func first(self: *(i32, bool)) -> i32 { (*self).0 } } \
        func main() -> bool { \
            let pair = (1, true); \
            let first = pair.first(); \
            let flag = pair.1; \
            flag.flag() \
        }";
    check_clean(source);
}

#[test]
fn a_transitive_supertrait_method_is_visible_from_the_leaf_trait() {
    let source = "\
        trait Root { func root(self: *Self) -> i32; } \
        trait Mid: Root { } \
        trait Leaf: Mid { } \
        func read<T: Leaf>(value: *T) -> i32 { value.root() }";
    check_clean(source);
}

#[test]
fn self_in_a_generic_trait_impl_method_is_the_implementing_type() {
    let source = "\
        struct Wrapper<T> { value: T } \
        trait Same { func same(self: Self) -> Self; } \
        impl<T> Same for Wrapper<T> { func same(self: Wrapper<T>) -> Wrapper<T> { self } } \
        func main() -> i32 { \
            let wrapper = Wrapper { value: 1 }; \
            let copy = wrapper.same(); \
            copy.value \
        }";
    check_clean(source);
}

#[test]
fn a_subtrait_method_is_callable_through_the_supertrait_relation() {
    let source = "\
        trait Base { func base(self: *Self) -> i32; } \
        trait Child: Base { } \
        struct Point { x: i32 } \
        impl Base for Point { func base(self: *Point) -> i32 { (*self).x } } \
        impl Child for Point { } \
        func read<T: Child>(value: *T) -> i32 { value.base() }";
    check_clean(source);
}

#[test]
fn trait_methods_resolve_statically_to_the_implementation() {
    let (fixture, hir, results) = check_clean(&format!(
        "{SHAPE} func main() -> i32 {{ let circle = Circle {{ radius: 3 }}; circle.area() }}"
    ));
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("area is resolved");
    assert!(matches!(resolved.kind, MethodKind::Trait { .. }));
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::SharedBorrow);
}

#[test]
fn a_trait_associated_function_has_a_signature() {
    let (fixture, hir, results) = check_clean(
        "struct Circle { radius: i32 } \
         trait Shape { func unit() -> i32; } \
         impl Shape for Circle { func unit() -> i32 { 7 } } \
         func main() -> i32 { Circle::unit() }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Call { callee, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a call");
    };
    let ExprKind::Path {
        res: seal_hir::Res::Function(function),
        ..
    } = hir.expr(callee).kind
    else {
        panic!("expected a function path");
    };
    let signature = results
        .signature(function)
        .expect("the associated function has a signature");
    assert!(signature.params.is_empty());
    assert_eq!(
        signature.ret,
        results.signature(function).expect("signed").ret
    );
}

#[test]
fn an_inherent_method_shadows_a_trait_method() {
    let source = "\
struct Circle { radius: i32 } \
trait Shape { func size(self: *Self) -> i32; } \
impl Shape for Circle { func size(self: *Circle) -> i32 { 1 } } \
impl Circle { func size(self: *Circle) -> i32 { 2 } } \
func main() -> i32 { let circle = Circle { radius: 1 }; circle.size() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("size is resolved");
    assert!(matches!(resolved.kind, MethodKind::User { .. }));
}

#[test]
fn two_traits_with_the_same_method_name_are_ambiguous() {
    let source = "\
struct Circle { radius: i32 } \
trait Shape { func size(self: *Self) -> i32; } \
trait Area { func size(self: *Self) -> i32; } \
impl Shape for Circle { func size(self: *Circle) -> i32 { 1 } } \
impl Area for Circle { func size(self: *Circle) -> i32 { 2 } } \
func main() -> i32 { let circle = Circle { radius: 1 }; circle.size() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2028", "declared by more than one trait");
}

#[test]
fn an_ambiguous_trait_associated_function_is_reported() {
    let source = "\
struct Circle { radius: i32 } \
trait Shape { func unit() -> i32; } \
trait Area { func unit() -> i32; } \
impl Shape for Circle { func unit() -> i32 { 1 } } \
impl Area for Circle { func unit() -> i32 { 2 } } \
func main() -> i32 { Circle::unit() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2028", "declared by more than one trait");
}

#[test]
fn a_mutable_receiver_reborrows_shared_for_a_read_only_trait_method() {
    let source = "\
struct Counter { value: i32 } \
trait Peak { func peak(self: *Self) -> i32; } \
impl Peak for Counter { func peak(self: *Counter) -> i32 { (*self).value } } \
func main() -> i32 { \
    let mut counter = Counter { value: 41 }; \
    let pointer = &mut counter; \
    pointer.peak() \
}";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("peak is resolved");
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::SharedReborrow);
    assert!(matches!(
        resolved.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Shared,
            ..
        }
    ));
}

#[test]
fn a_shared_receiver_cannot_call_an_exclusive_trait_method() {
    let source = "\
struct Counter { value: i32 } \
trait Bump { func bump(self: *mut Self); } \
impl Bump for Counter { func bump(self: *mut Counter) { (*self).value += 1; } } \
func main() { \
    let mut counter = Counter { value: 0 }; \
    let pointer = &counter; \
    pointer.bump(); \
}";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3031", "expects `*mut Counter`");
}

#[test]
fn an_inherent_method_still_wins_over_a_trait_associated_function() {
    let source = "\
struct Circle { radius: i32 } \
trait Shape { func unit() -> i32; } \
impl Shape for Circle { func unit() -> i32 { 1 } } \
impl Circle { func unit() -> i32 { 2 } } \
func main() -> i32 { Circle::unit() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Call { callee, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a call");
    };
    let ExprKind::Path {
        res: seal_hir::Res::Function(function),
        ..
    } = hir.expr(callee).kind
    else {
        panic!("expected a function path");
    };
    assert_eq!(
        hir.definitions().get(function).parent,
        Some(
            hir.definitions()
                .iter_enumerated()
                .find(|(_, data)| data.kind == seal_def::DefKind::Struct)
                .map(|(id, _)| id)
                .expect("Circle exists")
        )
    );
    assert!(results.signature(function).is_some());
}

#[test]
fn a_value_receiver_rejects_a_reference_receiver() {
    let source = "\
struct Token { value: i32 } \
trait Consume { func consume(self: Self) -> i32; } \
impl Consume for Token { func consume(self: Token) -> i32 { self.value } } \
func main() -> i32 { let token = Token { value: 1 }; let view = &token; view.consume() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3031", "expects `Token`");
}

#[test]
fn a_value_receiver_cannot_move_out_of_a_reference() {
    let source = "\
struct Token { value: i32 } \
trait Consume { func consume(self: Self) -> i32; } \
impl Consume for Token { func consume(self: Token) -> i32 { self.value } } \
func take(p: *mut Token) -> i32 { (*p).consume() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn a_value_receiver_cannot_move_out_of_a_slice() {
    let source = "\
struct Token { value: i32 } \
trait Consume { func consume(self: Self) -> i32; } \
impl Consume for Token { func consume(self: Token) -> i32 { self.value } } \
func first(tokens: *[Token]) -> i32 { tokens[0].consume() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn a_shared_receiver_borrows_a_slice_element_instead_of_moving_it() {
    check_clean(
        "struct Token { value: i32 }\n\
         trait View { func view(self: *Self) -> i32; }\n\
         impl View for Token { func view(self: *Token) -> i32 { (*self).value } }\n\
         func first(tokens: *[Token]) -> i32 { tokens[0].view() }",
    );
}

#[test]
fn a_shared_receiver_borrows_an_explicit_dereference_instead_of_moving_it() {
    check_clean(
        "struct Token { value: i32 }\n\
         trait View { func view(self: *Self) -> i32; }\n\
         impl View for Token { func view(self: *Token) -> i32 { (*self).value } }\n\
         func peek(token: *mut Token) -> i32 { (*token).view() }",
    );
}

#[test]
fn a_value_receiver_cannot_move_out_of_an_array_behind_a_reference() {
    for reference in ["*[Token; 2]", "*mut [Token; 2]"] {
        let source = "struct Token { value: i32 } \
             trait Consume { func consume(self: Self) -> i32; } \
             impl Consume for Token { func consume(self: Token) -> i32 { self.value } } \
             func first(tokens: REF) -> i32 { tokens[0].consume() }";
        let (fixture, _, _) = check_errors(&source.replace("REF", reference));
        fixture.expect("E3018", "cannot move out of a reference");
    }
}

#[test]
fn a_shared_receiver_borrows_an_array_element_behind_a_reference() {
    for reference in ["*[Token; 2]", "*mut [Token; 2]"] {
        let source = "struct Token { value: i32 } \
             trait View { func view(self: *Self) -> i32; } \
             impl View for Token { func view(self: *Token) -> i32 { (*self).value } } \
             func first(tokens: REF) -> i32 { tokens[0].view() }";
        check_clean(&source.replace("REF", reference));
    }
}

#[test]
fn a_value_receiver_cannot_move_a_field_out_of_a_dereference() {
    let source = "\
struct Token { value: i32 } \
struct Holder { token: Token } \
trait Consume { func consume(self: Self) -> i32; } \
impl Consume for Token { func consume(self: Token) -> i32 { self.value } } \
func take(holder: *mut Holder) -> i32 { (*holder).token.consume() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn a_shared_receiver_borrows_a_field_of_a_dereference() {
    check_clean(
        "struct Token { value: i32 }\n\
         struct Holder { token: Token }\n\
         trait View { func view(self: *Self) -> i32; }\n\
         impl View for Token { func view(self: *Token) -> i32 { (*self).value } }\n\
         func peek(holder: *mut Holder) -> i32 { (*holder).token.view() }",
    );
}

#[test]
fn a_value_receiver_cannot_move_out_of_a_nested_dereference() {
    let source = "\
struct Token { value: i32 } \
trait Consume { func consume(self: Self) -> i32; } \
impl Consume for Token { func consume(self: Token) -> i32 { self.value } } \
func take(token: *mut *mut Token) -> i32 { (**token).consume() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn a_shared_receiver_borrows_through_a_nested_dereference() {
    check_clean(
        "struct Token { value: i32 }\n\
         trait View { func view(self: *Self) -> i32; }\n\
         impl View for Token { func view(self: *Token) -> i32 { (*self).value } }\n\
         func peek(token: *mut *mut Token) -> i32 { (**token).view() }",
    );
}

#[test]
fn a_value_receiver_cannot_move_a_field_out_of_a_slice_element() {
    let source = "\
struct Token { value: i32 } \
struct Holder { token: Token } \
trait Consume { func consume(self: Self) -> i32; } \
impl Consume for Token { func consume(self: Token) -> i32 { self.value } } \
func first(holders: *[Holder]) -> i32 { holders[0].token.consume() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn a_shared_receiver_borrows_a_field_of_a_slice_element() {
    check_clean(
        "struct Token { value: i32 }\n\
         struct Holder { token: Token }\n\
         trait View { func view(self: *Self) -> i32; }\n\
         impl View for Token { func view(self: *Token) -> i32 { (*self).value } }\n\
         func first(holders: *[Holder]) -> i32 { holders[0].token.view() }",
    );
}

#[test]
fn an_exclusive_receiver_borrows_a_mutable_slice_element() {
    check_clean(
        "struct Token { value: i32 }\n\
         trait Bump { func bump(self: *mut Self); }\n\
         impl Bump for Token { func bump(self: *mut Token) { (*self).value += 1; } }\n\
         func first(tokens: *mut [Token]) { tokens[0].bump() }",
    );
}

#[test]
fn an_exclusive_receiver_cannot_write_through_a_shared_slice() {
    let source = "\
struct Token { value: i32 } \
trait Bump { func bump(self: *mut Self); } \
impl Bump for Token { func bump(self: *mut Token) { (*self).value += 1; } } \
func first(tokens: *[Token]) { tokens[0].bump() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3029", "needs to modify");
}

#[test]
fn a_default_body_calls_its_own_trait_method_symbolically() {
    let source = "\
        trait Shape { \
            func area(self: *Self) -> i32; \
            func twice(self: *Self) -> i32 { self.area() } \
        } \
        struct Square { side: i32 } \
        impl Shape for Square { \
            func area(self: *Square) -> i32 { (*self).side * (*self).side } \
        } \
        func main() -> i32 { \
            let square = Square { side: 3 }; \
            square.twice() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let twice = trait_method_named(&fixture, &hir, "twice");
    let area = trait_method_named(&fixture, &hir, "area");
    let body = hir.body_of(twice).expect("twice has a default body");
    let resolved = results
        .method(tail(&hir, body))
        .expect("the inner area call is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::TraitBound { method, .. } if method == area
    ));

    let (_, main) = function(&fixture, &hir, "main");
    let outer = results
        .method(tail(&hir, main))
        .expect("twice is resolved on Square");
    assert!(matches!(
        outer.kind,
        MethodKind::TraitBound { method, .. } if method == twice
    ));
}

#[test]
fn a_concrete_call_uses_an_omitted_default_method() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { func area(self: *Self) -> i32 { 7 } } \
        impl Shape for Circle {} \
        func main() -> i32 { \
            let circle = Circle { radius: 1 }; \
            circle.area() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let area = trait_method_named(&fixture, &hir, "area");
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("area is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::TraitBound { method, .. } if method == area
    ));
    assert_eq!(
        super::support::ty_text_with(&fixture, &hir, super::support::expr_ty(&results, call)),
        "i32"
    );
    let instance = results.instantiation(call).expect("instance is recorded");
    assert_eq!(instance.def, area);
    assert_eq!(hir.ty_ctxt().substs_args(instance.args).len(), 1);
}

#[test]
fn an_empty_impl_reaches_every_default_method() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { \
            func area(self: *Self) -> i32 { 7 } \
            func perimeter(self: *Self) -> i32 { 8 } \
        } \
        impl Shape for Circle {} \
        func main() -> i32 { \
            let circle = Circle { radius: 1 }; \
            circle.area() + circle.perimeter() \
        }";
    check_clean(source);
}

#[test]
fn an_override_of_a_default_method_stays_concrete() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { func area(self: *Self) -> i32 { 7 } } \
        impl Shape for Circle { func area(self: *Circle) -> i32 { 8 } } \
        func main() -> i32 { \
            let circle = Circle { radius: 1 }; \
            circle.area() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("area is resolved");
    assert!(matches!(resolved.kind, MethodKind::Trait { .. }));
}

#[test]
fn an_inherent_method_shadows_a_default_method() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { func area(self: *Self) -> i32 { 7 } } \
        impl Circle { func area(self: *Circle) -> i32 { 9 } } \
        impl Shape for Circle {} \
        func main() -> i32 { \
            let circle = Circle { radius: 1 }; \
            circle.area() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("area is resolved");
    assert!(matches!(resolved.kind, MethodKind::User { .. }));
}

#[test]
fn two_default_methods_with_the_same_name_are_ambiguous() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Left { func area(self: *Self) -> i32 { 1 } } \
        trait Right { func area(self: *Self) -> i32 { 2 } } \
        impl Left for Circle {} \
        impl Right for Circle {} \
        func main() -> i32 { \
            let circle = Circle { radius: 1 }; \
            circle.area() \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2028", "declared by more than one trait");
}

#[test]
fn a_default_method_from_a_supertrait_is_visible() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Base { func area(self: *Self) -> i32 { 3 } } \
        trait Shape: Base {} \
        impl Base for Circle {} \
        impl Shape for Circle {} \
        func main() -> i32 { \
            let circle = Circle { radius: 1 }; \
            circle.area() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let resolved = results.method(tail(&hir, body)).expect("area is resolved");
    assert!(matches!(resolved.kind, MethodKind::TraitBound { .. }));
}

#[test]
fn a_generic_default_method_substitutes_the_trait_arguments() {
    let source = "\
        struct Wrapper<T> { value: T } \
        trait Convert<T> { \
            func value(self: *Self) -> T; \
            func twice(self: *Self) -> T { self.value() } \
        } \
        impl<T: Copy> Convert<T> for Wrapper<T> { \
            func value(self: *Wrapper<T>) -> T { (*self).value } \
        } \
        func main() -> i32 { \
            let wrapper = Wrapper { value: 7 }; \
            wrapper.twice() \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("twice is resolved");
    assert!(matches!(resolved.kind, MethodKind::TraitBound { .. }));
    let instance = results.instantiation(call).expect("instance is recorded");
    assert_eq!(instance.def, trait_method_named(&fixture, &hir, "twice"));
    assert_eq!(hir.ty_ctxt().substs_args(instance.args).len(), 2);
}

#[test]
fn a_default_body_handles_exclusive_and_value_receivers() {
    let source = "\
        struct Item { value: i32 } \
        trait Counter { \
            func read(self: *Self) -> i32; \
            func bump(self: *mut Self) -> i32 { self.read() } \
        } \
        impl Counter for Item { \
            func read(self: *Item) -> i32 { (*self).value } \
        } \
        trait Consume { \
            func value(self: Self) -> i32; \
            func twice(self: Self) -> i32 { self.value() } \
        } \
        impl Consume for Item { \
            func value(self: Item) -> i32 { self.value } \
        } \
        func main() -> i32 { \
            let mut item = Item { value: 4 }; \
            let first = item.bump(); \
            let second = item.twice(); \
            first + second \
        }";
    check_clean(source);
}

#[test]
fn a_default_method_can_return_self() {
    let source = "\
        struct Item { value: i32 } \
        trait Duplicate { \
            func copy(self: *Self) -> Self; \
            func duplicate(self: *Self) -> Self { self.copy() } \
        } \
        impl Duplicate for Item { \
            func copy(self: *Item) -> Item { Item { value: (*self).value } } \
        } \
        func main() -> i32 { \
            let item = Item { value: 5 }; \
            let duplicate = item.duplicate(); \
            duplicate.value \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let tail = tail(&hir, body);
    let call = match &hir.expr(tail).kind {
        ExprKind::Field { base, .. } => *base,
        other => panic!("expected a field access, got {other:?}"),
    };
    assert_eq!(
        super::support::ty_text_with(&fixture, &hir, super::support::expr_ty(&results, call)),
        "Item"
    );
}

#[test]
fn a_default_method_from_a_supertrait_is_visible_through_a_bound() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Base { func area(self: *Self) -> i32 { 3 } } \
        trait Shape: Base {} \
        impl Base for Circle {} \
        impl Shape for Circle {} \
        func measure<T: Shape>(value: *T) -> i32 { value.area() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "measure");
    let resolved = results
        .method(tail(&hir, body))
        .expect("area is resolved through the bound");
    assert!(matches!(resolved.kind, MethodKind::TraitBound { .. }));
}

#[test]
fn a_default_body_reports_a_return_type_mismatch() {
    let fixture =
        super::support::Fixture::new("trait Shape { func area(self: *Self) -> i32 { true } }");
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_default_body_reports_an_unknown_method() {
    let fixture = super::support::Fixture::new(
        "trait Shape { func area(self: *Self) -> i32 { self.missing() } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3027", "no method named `missing`");
}

fn trait_method_named(fixture: &super::support::Fixture, hir: &seal_hir::Hir, name: &str) -> DefId {
    let symbol = fixture.interner().intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::TraitMethod && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no trait method named `{name}`"))
}
