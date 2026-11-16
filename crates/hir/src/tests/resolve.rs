//! Name-resolution tests.

use seal_def::DefKind;

use crate::{ExprKind, Res, StmtKind};

use super::support::{Fixture, expr_statement, function, let_binding, lower_clean, path_res, tail};

#[test]
fn functions_are_visible_before_declaration() {
    let (fixture, hir) = lower_clean("func main() { helper(); } func helper() {}");
    let (_, main) = function(&fixture, &hir, "main");
    let statement = hir.block(main.root).statements[0];
    let expr = expr_statement(&hir, statement);
    let callee = match hir.expr(expr).kind.clone() {
        ExprKind::Call { callee, .. } => callee,
        other => panic!("expected a call, got {other:?}"),
    };
    let definition = match path_res(&hir, callee) {
        Res::Function(definition) => definition,
        other => panic!("expected a function, got {other:?}"),
    };

    assert_eq!(fixture.symbol(hir.definitions().name(definition)), "helper");
    assert!(hir.body_of(definition).is_some());
}

#[test]
fn parameters_resolve_and_are_immutable() {
    let (fixture, hir) = lower_clean("func add(a: i32, b: i32) -> i32 { a + b }");
    let (function, body) = function(&fixture, &hir, "add");
    let ExprKind::Binary { lhs, rhs, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a binary expression");
    };

    for operand in [lhs, rhs] {
        let Res::Local(local) = path_res(&hir, operand) else {
            panic!("parameter must resolve to a local");
        };
        let data = hir.definitions().get(local);
        assert_eq!(data.kind, DefKind::Param);
        assert_eq!(data.parent, Some(function));
        assert!(!data.mutable);
    }
}

#[test]
fn lets_shadow_previous_bindings_in_the_same_scope() {
    let (fixture, hir) = lower_clean("func main() -> i32 { let x = 1; let x = 2; x }");
    let (_, body) = function(&fixture, &hir, "main");
    let statements = &hir.block(body.root).statements;
    let first = let_binding(&hir, statements[0]);
    let second = let_binding(&hir, statements[1]);

    let Res::Local(resolved) = path_res(&hir, tail(&hir, body)) else {
        panic!("tail must resolve to a local");
    };
    assert_eq!(resolved, second);
    assert_ne!(resolved, first);
}

#[test]
fn lets_shadow_parameters() {
    let (fixture, hir) = lower_clean("func f(x: i32) -> i32 { let x = 2; x }");
    let (_, body) = function(&fixture, &hir, "f");
    let parameter = body.params[0].def;
    let statement = hir.block(body.root).statements[0];
    let local = let_binding(&hir, statement);

    let Res::Local(resolved) = path_res(&hir, tail(&hir, body)) else {
        panic!("tail must resolve to a local");
    };
    assert_eq!(resolved, local);
    assert_ne!(resolved, parameter);
    assert_eq!(hir.definitions().kind(parameter), DefKind::Param);
    assert_eq!(hir.definitions().kind(local), DefKind::Binding);
}

#[test]
fn a_binding_is_not_visible_in_its_own_initializer() {
    let fixture = Fixture::new("func main() { let x = x; }");
    let hir = fixture.lower();

    fixture.expect("E2002", "cannot find `x`");
    let (_, body) = function(&fixture, &hir, "main");
    let statement = hir.block(body.root).statements[0];
    let init = match hir.stmt(statement).kind.clone() {
        StmtKind::Let { init, .. } => init.expect("binding has an initializer"),
        other => panic!("expected a let, got {other:?}"),
    };
    assert!(matches!(hir.expr(init).kind.clone(), ExprKind::Error));
}

#[test]
fn use_before_declaration_is_unresolved() {
    let fixture = Fixture::new("func main() { value; let value = 1; }");
    let hir = fixture.lower();

    fixture.expect("E2002", "cannot find `value`");
    let (_, body) = function(&fixture, &hir, "main");
    let statement = hir.block(body.root).statements[0];
    let expr = expr_statement(&hir, statement);
    assert!(matches!(hir.expr(expr).kind.clone(), ExprKind::Error));
}

#[test]
fn duplicate_functions_are_reported_but_both_are_lowered() {
    let fixture = Fixture::new("func main() {} func main() {}");
    let hir = fixture.lower();

    fixture.expect("E2001", "defined twice");
    assert_eq!(hir.bodies().len(), 2);
    assert_eq!(
        hir.definitions()
            .iter()
            .filter(|data| data.kind == DefKind::Function)
            .count(),
        2
    );
}

#[test]
fn duplicate_parameters_are_reported_but_both_are_lowered() {
    let fixture = Fixture::new("func f(x: i32, x: i32) {}");
    let hir = fixture.lower();

    fixture.expect("E2001", "parameter `x` is declared twice");
    let (_, body) = function(&fixture, &hir, "f");
    assert_eq!(body.params.len(), 2);
}

#[test]
fn assignment_targets_resolve_to_mutable_locals() {
    let (fixture, hir) = lower_clean("func main() { let mut x = 0; x = 1; x += 2; x++; }");
    let (_, body) = function(&fixture, &hir, "main");
    let statements = &hir.block(body.root).statements;
    let binding = let_binding(&hir, statements[0]);
    assert!(hir.definitions().is_mutable(binding));

    for statement in &statements[1..] {
        let expr = expr_statement(&hir, *statement);
        let target = match hir.expr(expr).kind.clone() {
            ExprKind::Assign { target, .. } | ExprKind::Update { target, .. } => target,
            other => panic!("expected an assignment or update, got {other:?}"),
        };
        let place = hir.place(target).expect("target must be a place");
        assert_eq!(place.local_def(), binding);
    }

    let last = expr_statement(&hir, statements[3]);
    let ExprKind::Update { postfix, .. } = hir.expr(last).kind.clone() else {
        panic!("expected an update");
    };
    assert!(postfix);
}

#[test]
fn assigning_to_a_function_is_left_to_the_type_checker() {
    let fixture = Fixture::new("func f() {} func main() { f = 1; }");
    let hir = fixture.lower();
    fixture.assert_clean();

    let (_, body) = function(&fixture, &hir, "main");
    let expr = expr_statement(&hir, hir.block(body.root).statements[0]);
    let ExprKind::Assign { target, .. } = hir.expr(expr).kind.clone() else {
        panic!("expected an assignment");
    };
    assert!(matches!(
        hir.expr(target).kind.clone(),
        ExprKind::Path(Res::Function(_))
    ));
    assert!(hir.place(target).is_none());
}

#[test]
fn break_and_continue_outside_a_loop_are_errors() {
    let fixture = Fixture::new("func main() { break; continue; }");
    let hir = fixture.lower();

    fixture.expect("E2003", "`break` outside of a loop");
    fixture.expect("E2004", "`continue` outside of a loop");
    let (_, body) = function(&fixture, &hir, "main");
    let statements = &hir.block(body.root).statements;
    assert!(matches!(
        hir.stmt(statements[0]).kind.clone(),
        StmtKind::Break
    ));
    assert!(matches!(
        hir.stmt(statements[1]).kind.clone(),
        StmtKind::Continue
    ));
}

#[test]
fn loop_control_inside_a_loop_is_clean() {
    let (fixture, hir) = lower_clean("func main() { while true { break; continue; } }");
    let (_, body) = function(&fixture, &hir, "main");
    let statement = hir.block(body.root).statements[0];
    let loop_body = match hir.stmt(statement).kind.clone() {
        StmtKind::While { body, .. } => body,
        other => panic!("expected a while, got {other:?}"),
    };
    let statements = &hir.block(loop_body).statements;
    assert!(matches!(
        hir.stmt(statements[0]).kind.clone(),
        StmtKind::Break
    ));
    assert!(matches!(
        hir.stmt(statements[1]).kind.clone(),
        StmtKind::Continue
    ));
}

#[test]
fn unresolved_call_callee_becomes_an_error_node() {
    let fixture = Fixture::new("func main() { missing(); }");
    let hir = fixture.lower();

    fixture.expect("E2002", "cannot find `missing`");
    let (_, body) = function(&fixture, &hir, "main");
    let expr = expr_statement(&hir, hir.block(body.root).statements[0]);
    let callee = match hir.expr(expr).kind.clone() {
        ExprKind::Call { callee, .. } => callee,
        other => panic!("expected a call, got {other:?}"),
    };
    assert!(matches!(hir.expr(callee).kind.clone(), ExprKind::Error));
}
