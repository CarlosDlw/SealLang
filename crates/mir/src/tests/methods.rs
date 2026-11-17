//! Intrinsic method calls in MIR: receivers, side effects, and chaining.

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
fn a_method_statement_keeps_its_side_effect() {
    let (fixture, hir, mir) = lower_clean("func main() { let mut a = [1, 2]; a.fill(7); }");
    let body = function(&fixture, &hir, &mir, "main");
    let calls = intrinsics(body);
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0].0, IntrinsicReceiver::Place(_)));
    assert_eq!(calls[0].1.len(), 1);
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
