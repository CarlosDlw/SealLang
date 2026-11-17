//! Intrinsic method calls in MIR: receivers, side effects, and chaining.

use seal_def::DefId;
use seal_ty::RefKind;

use crate::rvalue::{IntrinsicReceiver, Rvalue};

use super::support::{assignment, function, lower_clean};

fn intrinsics(body: &crate::body::Body) -> Vec<(&IntrinsicReceiver, &[crate::operand::Operand])> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            let (_, rvalue) = assignment(statement);
            match rvalue {
                Rvalue::Intrinsic { receiver, args, .. } => Some((receiver, args.as_slice())),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn user_methods_lower_to_direct_calls_with_the_receiver_first() {
    let source = "struct Point { x: i32, y: i32 } \
         impl Point { \
             func new(x: i32, y: i32) -> Point { Point { x: x, y: y } } \
             func sum(self: *Point) -> i32 { (*self).x + (*self).y } \
             func shift(self: *mut Point, dx: i32, dy: i32) { (*self).x += dx; (*self).y += dy; } \
         } \
         func main() -> i32 { \
             let mut point = Point::new(1, 2); \
             point.shift(3, 4); \
             point.sum() \
         }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");

    let shift = call_named(&fixture, &hir, body, "shift").expect("shift is called");
    assert_eq!(shift.1.len(), 3, "receiver plus two arguments");
    assert_eq!(shift.2.len(), 3, "each argument has a span");

    let sum = call_named(&fixture, &hir, body, "sum").expect("sum is called");
    assert_eq!(sum.1.len(), 1, "only the receiver");

    let borrows: Vec<&Rvalue> = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(|statement| assignment(statement).1)
        .filter(|rvalue| matches!(rvalue, Rvalue::Ref { .. }))
        .collect();
    assert_eq!(borrows.len(), 2, "one exclusive and one shared receiver");
    assert!(matches!(
        borrows[0],
        Rvalue::Ref {
            kind: RefKind::Exclusive,
            ..
        }
    ));
    assert!(matches!(
        borrows[1],
        Rvalue::Ref {
            kind: RefKind::Shared,
            ..
        }
    ));
}

#[test]
fn an_exact_reference_receiver_is_passed_as_a_value() {
    let source = "struct Token { value: i32 } \
         impl Token { \
             func read(self: *Token) -> i32 { (*self).value } \
         } \
         func main() -> i32 { \
             let token = Token { value: 41 }; \
             let view: *Token = &token; \
             view.read() + 1 \
         }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let call = call_named(&fixture, &hir, body, "read").expect("read is called");
    assert_eq!(call.1.len(), 1);

    let borrows = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| matches!(assignment(statement).1, Rvalue::Ref { .. }))
        .count();
    assert_eq!(borrows, 1, "only the written `&token` borrow exists");
}

#[test]
fn associated_calls_remain_ordinary_function_calls() {
    let source = "struct Token { value: i32 } \
         impl Token { func new(value: i32) -> Token { Token { value: value } } } \
         func main() -> i32 { let token = Token::new(41); token.value + 1 }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let call = call_named(&fixture, &hir, body, "new").expect("new is called");
    assert_eq!(call.1.len(), 1, "associated functions have no receiver");
}

#[test]
fn a_method_statement_keeps_its_side_effect() {
    let (fixture, hir, mir) = lower_clean("func main() { let mut a = [1, 2]; a.fill(7); }");
    let body = function(&fixture, &hir, &mir, "main");
    let calls = intrinsics(body);
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0].0, IntrinsicReceiver::Place(_)));
    assert_eq!(calls[0].1.len(), 1);
}

fn call_named<'a>(
    fixture: &super::support::Fixture,
    hir: &seal_hir::Hir,
    body: &'a crate::body::Body,
    name: &str,
) -> Option<(DefId, &'a [crate::operand::Operand], &'a [seal_span::Span])> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let (_, rvalue) = assignment(statement);
            let Rvalue::Call {
                target,
                args,
                arg_spans,
            } = rvalue
            else {
                return None;
            };
            let function = target.instance().def;
            (fixture.symbol(hir.definitions().name(function)) == name).then_some((
                function,
                args.as_slice(),
                arg_spans.as_slice(),
            ))
        })
}

#[test]
fn slice_receivers_are_fat_pointer_values() {
    let (fixture, hir, mir) = lower_clean("func f(v: *mut [i32]) { v.reverse(); }");
    let body = function(&fixture, &hir, &mir, "f");
    let calls = intrinsics(body);
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0].0, IntrinsicReceiver::Value(_)));
}

#[test]
fn chained_calls_lower_as_separate_intrinsics() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { (-5).abs().min(10) }");
    let body = function(&fixture, &hir, &mir, "main");
    assert_eq!(intrinsics(body).len(), 2);
}

#[test]
fn an_exclusive_reference_receiver_gets_a_deref_projection() {
    let (fixture, hir, mir) = lower_clean("func f(flag: *mut bool) { flag.toggle(); }");
    let body = function(&fixture, &hir, &mir, "f");
    let calls = intrinsics(body);
    assert_eq!(calls.len(), 1);
    let IntrinsicReceiver::Place(place) = calls[0].0 else {
        panic!("an exclusive scalar receiver must be a place");
    };
    assert!(place.is_deref(), "the reference must be dereferenced");
}
