//! Trait method dispatch, ambiguity, and shared reborrow.

use seal_hir::ExprKind;

use crate::{MethodKind, ReceiverAdjust};

use super::support::{check_clean, check_errors, function, tail};

const SHAPE: &str = "\
struct Circle { radius: i32 } \
trait Shape { func area(self: *Self) -> i32; } \
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius * (*self).radius } }";

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
