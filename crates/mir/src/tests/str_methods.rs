//! Inherent `str` method calls in MIR.

use crate::operand::Operand;
use crate::rvalue::{BinOp, CallTarget, Rvalue, StrViewKind};
use crate::terminator::TerminatorKind;
use seal_methods::IntrinsicOp;

use super::support::{
    assignment, function, local_named, lower_clean, lower_clean_in_core, ty_text,
};

#[test]
fn is_empty_lowers_to_a_direct_call_with_the_string_receiver() {
    let source = "\
        impl str { \
            public func is_empty(self: *str) -> bool { self.len() == 0 } \
        } \
        func main() -> bool { \
            let text: *str = \"hello\"; \
            text.is_empty() \
        }";
    let (fixture, hir, mir) = lower_clean_in_core(source);
    let body = function(&fixture, &hir, &mir, "main");
    let text = local_named(&fixture, body, "text");

    let (target, args) = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let (_, rvalue) = assignment(statement);
            let Rvalue::Call { target, args, .. } = rvalue else {
                return None;
            };
            let CallTarget::Direct { instance, .. } = target else {
                return None;
            };
            (fixture.symbol(hir.definitions().name(instance.def)) == "is_empty")
                .then_some((target, args.as_slice()))
        })
        .expect("is_empty must lower to a direct call");

    assert!(matches!(target, CallTarget::Direct { .. }));
    assert_eq!(args.len(), 1, "the receiver is the only argument");
    let Operand::Copy(place) = &args[0] else {
        panic!("the string receiver must be passed as a copied fat pointer");
    };
    assert_eq!(place.local_id(), Some(text));
}

#[test]
fn slice_builds_a_substring_after_range_and_boundary_checks() {
    let (fixture, hir, mir) = lower_clean(
        "func f(text: *str, start: usize, end: usize) -> *str { text.slice(start, end) }",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let view = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match assignment(statement).1 {
            Rvalue::StrView {
                kind,
                base,
                start,
                end,
                ty,
            } => Some((
                *kind,
                crate::validate::place_ty(&hir, body, base),
                start.is_some(),
                end.is_some(),
                *ty,
            )),
            _ => None,
        })
        .expect("slice must lower to a string view");
    assert_eq!(view.0, StrViewKind::Substring);
    assert_eq!(ty_text(&hir, view.1), "*str");
    assert!(view.2 && view.3, "slice keeps both byte bounds");
    assert_eq!(ty_text(&hir, view.4), "*str");

    assert_eq!(intrinsic_count(body, IntrinsicOp::StrLen), 1);
    assert_eq!(intrinsic_count(body, IntrinsicOp::StrIsCharBoundary), 2);
    assert_eq!(compare_count(body, BinOp::Le), 2);
    assert_eq!(panic_count(body), 4);
}

#[test]
fn as_bytes_builds_a_byte_view_without_checks() {
    let (fixture, hir, mir) = lower_clean("func f(text: *str) -> *[u8] { text.as_bytes() }");
    let body = function(&fixture, &hir, &mir, "f");
    let view = single_view(body);
    assert_eq!(view.0, StrViewKind::Bytes);
    assert!(!view.1 && !view.2, "as_bytes has no range");
    assert_eq!(ty_text(&hir, view.3), "*[u8]");
    assert_eq!(panic_count(body), 0);
}

#[test]
fn as_str_unchecked_builds_a_string_view_without_checks() {
    let (fixture, hir, mir) =
        lower_clean("func f(bytes: *[u8]) -> *str { unsafe { bytes.as_str_unchecked() } }");
    let body = function(&fixture, &hir, &mir, "f");
    let view = single_view(body);
    assert_eq!(view.0, StrViewKind::FromBytes);
    assert!(!view.1 && !view.2, "as_str_unchecked has no range");
    assert_eq!(ty_text(&hir, view.3), "*str");
    assert_eq!(panic_count(body), 0);
}

fn single_view(body: &crate::body::Body) -> (StrViewKind, bool, bool, seal_ty::TyId) {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match assignment(statement).1 {
            Rvalue::StrView {
                kind,
                start,
                end,
                ty,
                ..
            } => Some((*kind, start.is_some(), end.is_some(), *ty)),
            _ => None,
        })
        .expect("expected one string view")
}

fn intrinsic_count(body: &crate::body::Body, op: IntrinsicOp) -> usize {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(
                assignment(statement).1,
                Rvalue::Intrinsic { method, .. } if method.def().intrinsic == op
            )
        })
        .count()
}

fn compare_count(body: &crate::body::Body, op: BinOp) -> usize {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| matches!(assignment(statement).1, Rvalue::Binary { op: found, .. } if *found == op))
        .count()
}

fn panic_count(body: &crate::body::Body) -> usize {
    body.blocks
        .iter()
        .filter(|block| matches!(block.terminator.kind, TerminatorKind::Panic { .. }))
        .count()
}
