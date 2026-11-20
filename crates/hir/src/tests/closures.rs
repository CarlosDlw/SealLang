//! Closure lowering, capture resolution, and environment layout tests.

use seal_def::{DefId, DefKind};
use seal_ty::{IntTy, TyKind};

use crate::{BodyKind, CaptureDef, CaptureSource, ExprKind, Hir, Projection, StmtKind};

use super::support::{
    Fixture, assert_invariants, expr_statement, function, let_binding, lower_clean, tail,
};

/// Returns the first closure stored by a `let` in `body`.
fn closure_binding(hir: &Hir, body: &crate::Body) -> (DefId, Vec<CaptureDef>) {
    let statements = hir.block(body.root).statements.clone();
    for statement in statements {
        let StmtKind::Let {
            init: Some(init), ..
        } = hir.stmt(statement).kind.clone()
        else {
            continue;
        };
        if let ExprKind::Closure { closure, captures } = hir.expr(init).kind.clone() {
            return (closure, captures);
        }
    }
    panic!("the body stores no closure in a `let`");
}

fn closure_expr(hir: &Hir, expr: crate::HirExprId) -> (DefId, Vec<CaptureDef>) {
    match hir.expr(expr).kind.clone() {
        ExprKind::Closure { closure, captures } => (closure, captures),
        other => panic!("expected a closure, got {other:?}"),
    }
}

fn captures_of(hir: &Hir, expr: crate::HirExprId) -> Vec<CaptureDef> {
    closure_expr(hir, expr).1
}

#[test]
fn a_closure_without_captures_has_an_empty_environment() {
    let (fixture, hir) = lower_clean("func main() { let f = || 1; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let (closure, captures) = closure_binding(&hir, body);

    assert!(captures.is_empty());
    let definition = hir.closure(closure).expect("the closure has data");
    assert!(definition.captures.is_empty());
    assert!(!definition.is_move);
    assert!(hir.is_closure(closure));

    let closure_body = hir.body_of(closure).expect("the closure owns a body");
    assert_eq!(closure_body.kind, BodyKind::Closure);
    assert!(closure_body.params.is_empty());
    assert_eq!(closure_body.ret, None);
    assert!(matches!(
        &hir.expr(tail(&hir, closure_body)).kind,
        ExprKind::Lit(_)
    ));

    let adt = definition.adt;
    assert_eq!(hir.adt(closure), Some(adt));
    assert_eq!(hir.closure_for_adt(adt), Some(closure));
    assert!(hir.ty_ctxt().adt(adt).is_closure());
    assert!(hir.ty_ctxt().adt(adt).fields.is_empty());

    let main = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| fixture.symbol(data.name) == "main")
        .map(|(id, _)| id)
        .expect("the function exists");
    assert!(!hir.is_closure(main));
    assert!(hir.closure(main).is_none());
}

#[test]
fn a_captured_local_becomes_one_environment_field() {
    let (fixture, hir) = lower_clean("func main() -> i32 { let x = 1; let f = |y: i32| x + y; x }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let x = let_binding(&hir, hir.block(body.root).statements[0]);
    let (closure, captures) = closure_binding(&hir, body);

    assert_eq!(captures.len(), 1);
    let capture = captures[0];
    assert_eq!(capture.origin, x);
    assert_eq!(capture.source, CaptureSource::Local(x));
    assert_eq!(hir.definitions().kind(capture.field), DefKind::Field);
    assert_eq!(hir.definitions().kind(capture.binding), DefKind::Capture);
    assert_eq!(hir.definitions().get(capture.field).parent, Some(closure));
    assert_eq!(hir.definitions().get(capture.binding).parent, Some(closure));

    let definition = hir.closure(closure).expect("the closure has data");
    assert_eq!(definition.captures, captures);
    let aggregate = hir.ty_ctxt().adt(definition.adt);
    assert_eq!(aggregate.fields.len(), 1);
    assert_eq!(aggregate.fields[0].def, capture.field);

    let closure_body = hir.body_of(closure).expect("the closure owns a body");
    assert_eq!(closure_body.params.len(), 1);
    assert_eq!(
        hir.definitions().get(closure_body.params[0].def).parent,
        Some(closure)
    );
    assert_eq!(
        hir.definitions().get(closure_body.params[0].def).kind,
        DefKind::Param
    );

    let ExprKind::Binary { lhs, .. } = hir.expr(tail(&hir, closure_body)).kind.clone() else {
        panic!("expected a binary body");
    };
    let ExprKind::Capture {
        closure: owner,
        index,
    } = hir.expr(lhs).kind.clone()
    else {
        panic!("the captured value must not resolve to a path");
    };
    assert_eq!(owner, closure);
    assert_eq!(index, 0);
    assert_eq!(
        hir.place(lhs).map(|place| place.local_def()),
        Some(capture.binding)
    );
}

#[test]
fn captures_follow_first_mention_and_deduplicate() {
    let (fixture, hir) = lower_clean("func main() { let a = 1; let b = 2; let f = || b + a + b; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let statements = &hir.block(body.root).statements;
    let a = let_binding(&hir, statements[0]);
    let b = let_binding(&hir, statements[1]);

    let captures = captures_of(&hir, init_of(&hir, statements[2]));
    assert_eq!(captures.len(), 2);
    assert_eq!(captures[0].origin, b);
    assert_eq!(captures[1].origin, a);
}

#[test]
fn an_enclosing_closure_capture_uses_the_outer_source() {
    let (fixture, hir) =
        lower_clean("func main() { let x = 1; let f = |p: i32| |q: i32| x + p + q; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let x = let_binding(&hir, hir.block(body.root).statements[0]);
    let (outer, outer_captures) = closure_binding(&hir, body);

    // The outer closure carries the value its nested closure needs, even though
    // the outer body never mentions it directly.
    assert_eq!(outer_captures.len(), 1);
    assert_eq!(outer_captures[0].origin, x);
    assert_eq!(outer_captures[0].source, CaptureSource::Local(x));

    let outer_body = hir.body_of(outer).expect("the outer closure owns a body");
    let p = outer_body.params[0].def;
    let (inner, inner_captures) = closure_expr(&hir, tail(&hir, outer_body));
    assert_eq!(inner_captures.len(), 2);
    assert_eq!(inner_captures[0].origin, x);
    assert_eq!(inner_captures[0].source, CaptureSource::Outer(0));
    assert_eq!(inner_captures[1].origin, p);
    assert_eq!(inner_captures[1].source, CaptureSource::Local(p));

    let inner_body = hir.body_of(inner).expect("the inner closure owns a body");
    assert_eq!(inner_body.params.len(), 1);
    assert_eq!(
        hir.definitions().get(inner_body.params[0].def).parent,
        Some(inner)
    );
}

#[test]
fn a_nested_closure_propagates_through_each_environment() {
    let (fixture, hir) = lower_clean("func main() { let x = 1; let f = |p: i32| |q: i32| x + q; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let x = let_binding(&hir, hir.block(body.root).statements[0]);
    let (outer, outer_captures) = closure_binding(&hir, body);
    assert_eq!(outer_captures.len(), 1);
    assert_eq!(outer_captures[0].origin, x);

    let outer_body = hir.body_of(outer).expect("the outer closure owns a body");
    let inner_captures = captures_of(&hir, tail(&hir, outer_body));
    assert_eq!(inner_captures.len(), 1);
    assert_eq!(inner_captures[0].origin, x);
    assert_eq!(inner_captures[0].source, CaptureSource::Outer(0));
}

#[test]
fn shadowing_inside_the_body_is_not_a_capture() {
    let (fixture, hir) =
        lower_clean("func main() { let x = 1; let f = |y: i32| { let x = 2; x + y }; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let (closure, captures) = closure_binding(&hir, body);
    assert!(captures.is_empty());

    let closure_body = hir.body_of(closure).expect("the closure owns a body");
    let ExprKind::Block(block) = hir.expr(tail(&hir, closure_body)).kind.clone() else {
        panic!("expected a block body");
    };
    let inner = let_binding(&hir, hir.block(block).statements[0]);
    let ExprKind::Binary { lhs, .. } = hir
        .expr(hir.block(block).tail.expect("block value"))
        .kind
        .clone()
    else {
        panic!("expected a binary body");
    };
    assert_eq!(
        hir.place(lhs).expect("the operand is a place").local_def(),
        inner,
        "the shadowed binding is an ordinary local"
    );
    assert_eq!(hir.definitions().get(inner).parent, Some(closure));
}

#[test]
fn move_changes_only_the_recorded_flag() {
    let (fixture, plain) = lower_clean("func main() { let x = 1; let f = || x; }");
    assert_invariants(&fixture, &plain);
    let (_, body) = function(&fixture, &plain, "main");
    let (plain_closure, plain_captures) = closure_binding(&plain, body);

    let (fixture, moved) = lower_clean("func main() { let x = 1; let f = move || x; }");
    assert_invariants(&fixture, &moved);
    let (_, body) = function(&fixture, &moved, "main");
    let (moved_closure, moved_captures) = closure_binding(&moved, body);

    assert!(!plain.closure(plain_closure).expect("closure data").is_move);
    assert!(moved.closure(moved_closure).expect("closure data").is_move);
    assert_eq!(plain_captures.len(), moved_captures.len());
    assert_eq!(plain_captures[0].source, moved_captures[0].source);
    assert_eq!(plain_captures[0].origin, moved_captures[0].origin);
}

#[test]
fn constants_and_statics_are_not_captured() {
    let (fixture, hir) = lower_clean(
        "const G: i32 = 1;\n\
         func main() { let x = 1; let f = || { const K: i32 = 2; static S: i32 = 3; x + K }; }",
    );
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let x = let_binding(&hir, hir.block(body.root).statements[0]);
    let (_, captures) = closure_binding(&hir, body);
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].origin, x);
}

#[test]
fn a_static_initializer_still_rejects_captures_from_a_closure() {
    let fixture = Fixture::new("func main() { let x = 1; let f = || { static S: i32 = x; }; }");
    let hir = fixture.lower();
    fixture.expect("E3060", "cannot capture `x`");
    let (_, body) = function(&fixture, &hir, "main");
    let (closure, captures) = closure_binding(&hir, body);
    assert!(captures.is_empty());
    assert!(hir.body_of(closure).is_some());
}

#[test]
fn a_capture_supports_field_and_index_places() {
    let (fixture, hir) = lower_clean(
        "struct P { v: i32 }\n\
         func main() { let p = P { v: 1 }; let a = [1, 2];\
         let f = || p.v; let g = |i: i32| a[i]; }",
    );
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let statements = &hir.block(body.root).statements;

    let ExprKind::Closure {
        closure: field_closure,
        ..
    } = hir.expr(init_of(&hir, statements[2])).kind.clone()
    else {
        panic!("expected a closure");
    };
    let field_body = hir.body_of(field_closure).expect("the closure owns a body");
    let field_expr = tail(&hir, field_body);
    let place = hir.place(field_expr).expect("the field access is a place");
    assert_eq!(
        fixture.symbol(match place.projections()[0] {
            Projection::Field(name) => name,
            other => panic!("expected a field projection, got {other:?}"),
        }),
        "v"
    );
    assert_eq!(
        hir.place(match hir.expr(field_expr).kind.clone() {
            ExprKind::Field { base, .. } => base,
            other => panic!("expected a field access, got {other:?}"),
        })
        .expect("the capture is a place")
        .local_def(),
        hir.closure(field_closure).expect("closure data").captures[0].binding
    );

    let ExprKind::Closure {
        closure: index_closure,
        ..
    } = hir.expr(init_of(&hir, statements[3])).kind.clone()
    else {
        panic!("expected a closure");
    };
    let index_body = hir.body_of(index_closure).expect("the closure owns a body");
    let index_expr = tail(&hir, index_body);
    let place = hir.place(index_expr).expect("the index is a place");
    assert!(matches!(place.projections(), [Projection::Index(_)]));
}

#[test]
fn a_capture_supports_tuple_field_places() {
    let (fixture, hir) = lower_clean("func main() { let t = (1, 2); let f = || t.0; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let (closure, captures) = closure_binding(&hir, body);
    assert_eq!(captures.len(), 1);

    let closure_body = hir.body_of(closure).expect("the closure owns a body");
    let tuple = tail(&hir, closure_body);
    let place = hir.place(tuple).expect("the tuple field is a place");
    assert!(matches!(place.projections(), [Projection::TupleField(0)]));
    assert_eq!(place.local_def(), captures[0].binding);
}

#[test]
fn an_unannotated_parameter_keeps_the_error_placeholder() {
    let (fixture, hir) = lower_clean("func main() { let f = |y| y; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let (closure, _) = closure_binding(&hir, body);
    let closure_body = hir.body_of(closure).expect("the closure owns a body");
    assert_eq!(closure_body.params.len(), 1);
    assert_eq!(hir.ty_ctxt().kind(closure_body.params[0].ty), TyKind::Error);
}

#[test]
fn an_annotated_result_is_lowered() {
    let (fixture, hir) = lower_clean("func main() { let f = || -> i32 { 1 }; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let (closure, _) = closure_binding(&hir, body);
    let closure_body = hir.body_of(closure).expect("the closure owns a body");
    assert_eq!(
        hir.ty_ctxt()
            .kind(closure_body.ret.expect("annotated result")),
        TyKind::Int(IntTy::I32)
    );
}

#[test]
fn every_closure_body_is_owned_once() {
    let (fixture, hir) = lower_clean("func main() { let f = || || 1; let g = |x: i32| x; }");
    assert_invariants(&fixture, &hir);
    let closures: Vec<DefId> = hir
        .bodies()
        .iter()
        .filter(|body| body.kind == BodyKind::Closure)
        .map(|body| body.owner)
        .collect();
    assert_eq!(closures.len(), 3);
    for closure in closures {
        assert!(hir.is_closure(closure));
        assert_eq!(hir.body_of(closure).map(|body| body.owner), Some(closure));
        assert_eq!(hir.definitions().kind(closure), DefKind::Closure);
    }
}

/// Returns the initializer of a `let` statement.
fn init_of(hir: &Hir, statement: crate::HirStmtId) -> crate::HirExprId {
    match hir.stmt(statement).kind.clone() {
        StmtKind::Let { init, .. } => init.expect("binding has an initializer"),
        other => panic!("expected a let, got {other:?}"),
    }
}

#[test]
fn variables_capture_as_a_whole_place() {
    let (fixture, hir) = lower_clean(
        "struct S { a: i32 }\n\
         func main() { let s = S { a: 1 }; let f = || s.a + s.a; }",
    );
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let (_, captures) = closure_binding(&hir, body);
    assert_eq!(captures.len(), 1, "field accesses capture the whole value");
}

#[test]
fn an_unused_expression_statement_of_a_closure_lowers() {
    let (fixture, hir) = lower_clean("func main() { || 1; }");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    let expr = expr_statement(&hir, hir.block(body.root).statements[0]);
    assert!(matches!(&hir.expr(expr).kind, ExprKind::Closure { .. }));
}
