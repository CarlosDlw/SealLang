//! The language examples must parse into the expected shapes.

use std::path::PathBuf;

use seal_ast::{ExprKind, Literal, StmtKind};

use crate::tests::support::{body, parse_clean, sole_function, tail};

fn example(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn all_examples_parse_without_diagnostics() {
    for name in ["0001_main.sl", "0002_variables.sl", "0003_operators.sl"] {
        let source = example(name);
        let (fixture, ast) = parse_clean(&source);
        assert_eq!(ast.program().len(), 1, "{name} declares one function");
        let declaration = sole_function(&ast);
        assert_eq!(fixture.symbol(declaration.name), "main", "{name}");
        assert_eq!(declaration.params.len(), 0, "{name}");
        assert!(declaration.ret.is_some(), "{name} declares i32");
    }
}

#[test]
fn main_example_returns_zero_as_its_tail() {
    let source = example("0001_main.sl");
    let (_, ast) = parse_clean(&source);
    let declaration = sole_function(&ast);
    assert!(matches!(
        tail(&ast, declaration).kind,
        ExprKind::Lit(Literal::Int { value: 0, .. })
    ));
}

#[test]
fn variables_example_has_bindings_and_a_tail_name() {
    let source = example("0002_variables.sl");
    let (fixture, ast) = parse_clean(&source);
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 5);
    for statement in &block.statements {
        assert!(matches!(ast.stmt(*statement).kind, StmtKind::Let(_)));
    }
    let tail = tail(&ast, declaration);
    match tail.kind {
        ExprKind::Path(symbol) => assert_eq!(fixture.symbol(symbol), "c"),
        ref other => panic!("expected path tail, got {other:?}"),
    }
}

#[test]
fn operators_example_has_expression_statements_and_a_tail() {
    let source = example("0003_operators.sl");
    let (fixture, ast) = parse_clean(&source);
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert!(block.statements.len() > 15);
    assert!(block.tail.is_some());
    let tail = tail(&ast, declaration);
    match tail.kind {
        ExprKind::Path(symbol) => assert_eq!(fixture.symbol(symbol), "b1"),
        ref other => panic!("expected path tail, got {other:?}"),
    }
    assert!(fixture.file.text().contains("+="));
}
